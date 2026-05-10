using LibreHardwareMonitor.Hardware;

namespace SysPulse.HwHelper;

/// <summary>
/// Convert the live LHM <see cref="Computer"/> tree into a flat <see cref="HwSnapshot"/>
/// DTO that the Rust side can consume. Each per-sensor read is wrapped so that
/// a single failure cannot block the entire snapshot.
///
/// WMI queries (disk metadata, memory modules) are NOT performed here — they
/// live in <see cref="WmiCache"/> and are populated once at startup. This keeps
/// the hot-path snapshot under the Rust-side request timeout.
/// </summary>
public static class SnapshotBuilder
{
    public static HwSnapshot Build(ComputerHost host, WmiCache wmi)
    {
        host.Refresh();

        var snap = new HwSnapshot
        {
            TimestampMs = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
        };

        int gpuIndex = 0;
        int diskIndex = 0;

        foreach (IHardware hw in host.Inner.Hardware)
        {
            switch (hw.HardwareType)
            {
                case HardwareType.Cpu:
                    snap.Cpu = TryReadCpu(hw);
                    break;

                case HardwareType.GpuNvidia:
                case HardwareType.GpuAmd:
                case HardwareType.GpuIntel:
                    snap.Gpus.Add(TryReadGpu(hw, gpuIndex++));
                    break;

                case HardwareType.Memory:
                    snap.Memory = TryReadMemory(hw, wmi);
                    break;

                case HardwareType.Storage:
                    snap.Disks.Add(TryReadDisk(hw, diskIndex++));
                    break;

                case HardwareType.Motherboard:
                    snap.Motherboard = TryReadMotherboard(hw);
                    break;

                case HardwareType.SuperIO:
                    // SuperIO often appears as sub-hardware of Motherboard, but on
                    // some boards it's a top-level node — surface its fans there.
                    AppendFans(hw, snap.Fans);
                    break;
            }

            // Fans can live as sub-hardware of any of the above.
            foreach (IHardware sub in hw.SubHardware)
            {
                AppendFans(sub, snap.Fans);
            }
        }

        ApplySystemDriveSpace(snap.Disks);
        ApplyDiskMetadata(snap.Disks, wmi);

        return snap;
    }

    // -----------------------------------------------------------------------
    // Per-hardware readers — every sensor probe is wrapped in TryRead so a bad
    // value does not zero out neighbouring fields. CP-1.1 / CP-4.2 / CP-5.2.
    // -----------------------------------------------------------------------

    private static CpuHw TryReadCpu(IHardware hw)
    {
        var cpu = new CpuHw { Name = hw.Name };

        var temps = hw.Sensors.Where(s => s.SensorType == SensorType.Temperature).ToList();
        cpu.PackageTempC = ValidTemp(
            temps.FirstOrDefault(s =>
                s.Name.Contains("Package", StringComparison.OrdinalIgnoreCase)
                || s.Name.Contains("CCD1", StringComparison.OrdinalIgnoreCase)
                || s.Name.Contains("Tctl", StringComparison.OrdinalIgnoreCase))?.Value);
        cpu.PerCoreTempsC = temps
            .Where(s => s.Name.StartsWith("Core ", StringComparison.OrdinalIgnoreCase)
                     || s.Name.StartsWith("CPU Core #", StringComparison.OrdinalIgnoreCase))
            .OrderBy(s => s.Index)
            .Select(s => ValidTemp(s.Value))
            .ToList();

        var loads = hw.Sensors.Where(s => s.SensorType == SensorType.Load).ToList();
        cpu.TotalUsage = (double)(loads.FirstOrDefault(s =>
            s.Name.Contains("Total", StringComparison.OrdinalIgnoreCase))?.Value ?? 0f);
        cpu.PerCoreUsage = loads
            .Where(s => s.Name.StartsWith("CPU Core #", StringComparison.OrdinalIgnoreCase)
                     || s.Name.StartsWith("Core #", StringComparison.OrdinalIgnoreCase))
            .OrderBy(s => s.Index)
            .Select(s => (double)(s.Value ?? 0f))
            .ToList();

        var clocks = hw.Sensors.Where(s => s.SensorType == SensorType.Clock).ToList();
        cpu.FrequencyMhz = (double?)clocks
            .Where(s => s.Name.StartsWith("Core #", StringComparison.OrdinalIgnoreCase)
                     || s.Name.StartsWith("CPU Core #", StringComparison.OrdinalIgnoreCase))
            .Select(s => s.Value ?? 0f)
            .DefaultIfEmpty(0f)
            .Max();
        if (cpu.FrequencyMhz <= 0) cpu.FrequencyMhz = null;

        var powers = hw.Sensors.Where(s => s.SensorType == SensorType.Power).ToList();
        cpu.PowerW = (double?)powers
            .FirstOrDefault(s => s.Name.Contains("Package", StringComparison.OrdinalIgnoreCase))
            ?.Value;

        return cpu;
    }

    private static GpuHw TryReadGpu(IHardware hw, int index)
    {
        var vendor = hw.HardwareType switch
        {
            HardwareType.GpuNvidia => "nvidia",
            HardwareType.GpuAmd => "amd",
            HardwareType.GpuIntel => "intel",
            _ => "unknown",
        };

        var gpu = new GpuHw { Index = index, Name = hw.Name, Vendor = vendor };

        gpu.UsagePercent = (double?)hw.Sensors
            .FirstOrDefault(s => s.SensorType == SensorType.Load
                              && s.Name.Contains("GPU Core", StringComparison.OrdinalIgnoreCase))
            ?.Value;

        gpu.MemUsedMb = (double?)hw.Sensors
            .FirstOrDefault(s => s.SensorType == SensorType.SmallData
                              && s.Name.Contains("Used", StringComparison.OrdinalIgnoreCase))
            ?.Value;
        gpu.MemTotalMb = (double?)hw.Sensors
            .FirstOrDefault(s => s.SensorType == SensorType.SmallData
                              && s.Name.Contains("Total", StringComparison.OrdinalIgnoreCase))
            ?.Value;

        gpu.TempC = ValidTemp(hw.Sensors
            .FirstOrDefault(s => s.SensorType == SensorType.Temperature
                              && s.Name.Contains("GPU Core", StringComparison.OrdinalIgnoreCase))
            ?.Value
            ?? hw.Sensors
                .FirstOrDefault(s => s.SensorType == SensorType.Temperature)
                ?.Value);

        gpu.PowerW = (double?)hw.Sensors
            .FirstOrDefault(s => s.SensorType == SensorType.Power)
            ?.Value;

        var fanSensor = hw.Sensors.FirstOrDefault(s => s.SensorType == SensorType.Fan);
        gpu.FanRpm = ValidRpm(fanSensor?.Value);
        gpu.FanPwm = (double?)hw.Sensors
            .FirstOrDefault(s => s.SensorType == SensorType.Control)
            ?.Value;

        return gpu;
    }

    private static MemoryHw TryReadMemory(IHardware hw, WmiCache wmi)
    {
        var mem = new MemoryHw();

        var dataUsed = hw.Sensors.FirstOrDefault(s => s.SensorType == SensorType.Data
            && s.Name.Contains("Used", StringComparison.OrdinalIgnoreCase));
        var dataAvail = hw.Sensors.FirstOrDefault(s => s.SensorType == SensorType.Data
            && s.Name.Contains("Available", StringComparison.OrdinalIgnoreCase));
        var loadMem = hw.Sensors.FirstOrDefault(s => s.SensorType == SensorType.Load
            && s.Name.Contains("Memory", StringComparison.OrdinalIgnoreCase));

        if (dataUsed?.Value is float u && dataAvail?.Value is float a)
        {
            // LHM Data sensors are in GB.
            const double gbToBytes = 1024d * 1024d * 1024d;
            ulong used = (ulong)(u * gbToBytes);
            ulong total = (ulong)((u + a) * gbToBytes);
            mem.UsedBytes = used;
            mem.TotalBytes = total;
        }
        mem.UsedPercent = (double)(loadMem?.Value ?? 0f);

        // Use cached WMI data instead of querying on every snapshot.
        mem.Modules = wmi.MemoryModules;
        mem.FrequencyMhz = wmi.MemoryFrequencyMhz;
        mem.Channels = wmi.MemoryChannels;

        return mem;
    }

    private static DiskHw TryReadDisk(IHardware hw, int index)
    {
        var disk = new DiskHw
        {
            Index = index,
            Model = hw.Name,
            Bus = InferDiskBus(hw),
            Identifier = hw.Identifier.ToString(),
        };

        var temp = hw.Sensors.FirstOrDefault(s => s.SensorType == SensorType.Temperature);
        disk.TempC = ValidTemp(temp?.Value);

        var read = hw.Sensors.FirstOrDefault(s => s.SensorType == SensorType.Throughput
            && s.Name.Contains("Read", StringComparison.OrdinalIgnoreCase));
        var write = hw.Sensors.FirstOrDefault(s => s.SensorType == SensorType.Throughput
            && s.Name.Contains("Write", StringComparison.OrdinalIgnoreCase));
        disk.ReadBytesPerSec = (double?)read?.Value;
        disk.WriteBytesPerSec = (double?)write?.Value;

        // Health: read remaining-life as a proxy when available.
        var life = hw.Sensors.FirstOrDefault(s =>
            s.SensorType == SensorType.Level
            && s.Name.Contains("Remaining Life", StringComparison.OrdinalIgnoreCase));
        if (life?.Value is float lifePct)
        {
            disk.Health = lifePct switch
            {
                >= 50f => "good",
                >= 20f => "warning",
                _ => "critical",
            };
        }

        return disk;
    }

    private static void ApplySystemDriveSpace(List<DiskHw> disks)
    {
        if (disks.Count == 0) return;

        try
        {
            var systemPath = Environment.GetFolderPath(Environment.SpecialFolder.System);
            var root = Path.GetPathRoot(systemPath);
            if (string.IsNullOrWhiteSpace(root)) return;

            var drive = new DriveInfo(root);
            if (!drive.IsReady || drive.TotalSize <= 0) return;

            var total = (ulong)drive.TotalSize;
            var free = (ulong)Math.Max(0, drive.AvailableFreeSpace);
            disks[0].TotalBytes = total;
            disks[0].UsedBytes = total >= free ? total - free : 0;
        }
        catch
        {
            /* drive space is optional */
        }
    }

    private static string InferDiskBus(IHardware hw)
    {
        var id = hw.Identifier.ToString().ToLowerInvariant();
        var name = hw.Name.ToLowerInvariant();
        if (id.Contains("nvme") || name.Contains("nvme")) return "nvme";
        if (id.Contains("usb") || name.Contains("usb")) return "usb";
        if (id.Contains("sata") || name.Contains("sata")) return "sata";
        if (id.Contains("hdd") || id.Contains("ssd")) return "sata";
        return "unknown";
    }

    private static void ApplyDiskMetadata(List<DiskHw> disks, WmiCache wmi)
    {
        if (disks.Count == 0) return;

        var metadataRows = new List<DiskMetadataRow>();
        metadataRows.AddRange(wmi.DiskDriveRows);
        metadataRows.AddRange(wmi.StorageDiskRows);
        if (metadataRows.Count == 0) return;

        foreach (var disk in disks)
        {
            var matches = metadataRows
                .Where(row =>
                    !string.IsNullOrWhiteSpace(row.Name)
                    && (disk.Model.Contains(row.Name, StringComparison.OrdinalIgnoreCase)
                        || row.Name.Contains(disk.Model, StringComparison.OrdinalIgnoreCase)))
                .ToList();

            if (matches.Count == 0 && disk.Index < metadataRows.Count) matches.Add(metadataRows[disk.Index]);
            if (matches.Count == 0) continue;

            var bus = matches.Select(row => row.Bus).FirstOrDefault(x => !string.IsNullOrWhiteSpace(x));
            if ((string.IsNullOrWhiteSpace(disk.Bus) || disk.Bus == "unknown")
                && !string.IsNullOrWhiteSpace(bus))
            {
                disk.Bus = bus;
            }

            var health = matches.Select(row => row.Health).FirstOrDefault(x => !string.IsNullOrWhiteSpace(x));
            if ((string.IsNullOrWhiteSpace(disk.Health) || disk.Health == "unknown")
                && !string.IsNullOrWhiteSpace(health))
            {
                disk.Health = health;
            }
        }
    }

    private static MotherboardHw TryReadMotherboard(IHardware hw)
    {
        var mb = new MotherboardHw
        {
            Vendor = "",
            Model = hw.Name,
        };

        // Super I/O usually shows up as sub-hardware.
        var superIo = hw.SubHardware.FirstOrDefault(h => h.HardwareType == HardwareType.SuperIO);
        if (superIo != null) mb.SuperIo = superIo.Name;

        IEnumerable<ISensor> AllSensors() =>
            hw.Sensors.Concat(hw.SubHardware.SelectMany(s => s.Sensors));

        foreach (var s in AllSensors())
        {
            switch (s.SensorType)
            {
                case SensorType.Temperature when ValidTemp(s.Value) is double t:
                    mb.TemperaturesC.Add(new NamedValue {
                        Name = s.Name, Value = t, Identifier = s.Identifier.ToString() });
                    break;
                case SensorType.Voltage when s.Value is float v && v >= 0 && v <= 15:
                    mb.VoltagesV.Add(new NamedValue {
                        Name = s.Name, Value = v, Identifier = s.Identifier.ToString() });
                    break;
            }
        }

        return mb;
    }

    private static void AppendFans(IHardware hw, List<FanHw> dest)
    {
        var fanSensors = hw.Sensors.Where(s => s.SensorType == SensorType.Fan).ToList();
        var ctrls = hw.Sensors.Where(s => s.SensorType == SensorType.Control).ToList();
        foreach (var f in fanSensors)
        {
            // Match a control sensor by index when possible.
            var pwm = ctrls.FirstOrDefault(c => c.Index == f.Index);
            dest.Add(new FanHw
            {
                Id = f.Identifier.ToString(),
                Name = $"{hw.Name} / {f.Name}",
                Rpm = ValidRpm(f.Value),
                PwmPercent = (double?)pwm?.Value,
                Controllable = (f.Control != null) || (pwm?.Control != null),
            });
        }
    }

    // CP-1.2: temperature ∈ [0, 150]
    private static double? ValidTemp(float? raw) => raw is float v && v >= 0f && v <= 150f ? v : null;

    // CP-6.1: rpm ∈ [0, 10000]
    private static double? ValidRpm(float? raw) => raw is float v && v >= 0f && v <= 10000f ? v : null;
}
