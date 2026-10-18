using System.Diagnostics;
using LibreHardwareMonitor.Hardware;
using LibreHardwareMonitor.Hardware.Storage;
using Reading = SysPulse.HwHelper.FanChannels.Reading;

namespace SysPulse.HwHelper;

/// <summary>Capture each sensor once; publish only successfully refreshed devices.</summary>
public static class SnapshotBuilder
{
    public static HwSnapshot Build(ComputerHost host, WmiCache wmi) => Build(host.Refresh(), wmi);

    public static HwSnapshot Build(IEnumerable<IHardware> refreshed, WmiCache wmi)
    {
        var snap = new HwSnapshot { TimestampMs = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() };
        foreach (var hw in refreshed)
        {
            try
            {
                var sensors = FanChannels.Capture(hw);
                var fans = ReadFans(hw, sensors);
                switch (hw.HardwareType)
                {
                    case HardwareType.Cpu:
                        var cpu = ReadCpu(hw.Name, sensors);
                        snap.Cpu = snap.Cpu is null ? cpu : MergeCpu(snap.Cpu, cpu);
                        break;
                    case HardwareType.GpuNvidia:
                    case HardwareType.GpuAmd:
                    case HardwareType.GpuIntel:
                        snap.Gpus.Add(ReadGpu(hw, sensors, fans, snap.Gpus.Count));
                        break;
                    case HardwareType.Memory:
                        snap.Memory = ReadMemory(sensors, wmi);
                        break;
                    case HardwareType.Storage:
                        snap.Disks.Add(ReadDisk(hw, sensors, wmi, snap.Disks.Count));
                        break;
                    case HardwareType.Motherboard:
                    case HardwareType.SuperIO:
                        snap.Motherboard ??= new MotherboardHw();
                        if (hw.HardwareType == HardwareType.Motherboard) snap.Motherboard.Model = hw.Name;
                        else snap.Motherboard.SuperIo = hw.Name;
                        foreach (var s in sensors)
                        {
                            if (s.Type == SensorType.Temperature && Range(s.Value, 0, 150) is double t)
                                snap.Motherboard.TemperaturesC.Add(new NamedValue { Name = s.Name, Identifier = s.Id, Value = t });
                            if (s.Type == SensorType.Voltage && Range(s.Value, 0, 15) is double v)
                                snap.Motherboard.VoltagesV.Add(new NamedValue { Name = s.Name, Identifier = s.Id, Value = v });
                        }
                        break;
                }
                snap.Fans.AddRange(fans);
            }
            catch (Exception ex) { Console.Error.WriteLine($"[hw-helper] device read failed: {ex.Message}"); }
        }
        return snap;
    }

    private static List<FanHw> ReadFans(IHardware hardware, List<Reading> sensors)
    {
        var fans = new List<FanHw>();
        foreach (var sensor in sensors.Where(s => s.Type == SensorType.Fan).OrderBy(s => s.Index))
        {
            var match = FanChannels.Resolve(hardware.HardwareType, sensors, sensor);
            fans.Add(new FanHw {
                Id = sensor.Id, Name = $"{hardware.Name} / {sensor.Name}", Rpm = FanChannels.Rpm(sensor.Value),
                PwmPercent = Range(match?.Pwm, 0, 100), Controllable = match != null,
            });
        }
        return fans;
    }

    public static double? Range(double? value, double min, double max) =>
        value is double v && double.IsFinite(v) && v >= min && v <= max ? v : null;
    private static bool Is(string value, string name) => value.Equals(name, StringComparison.OrdinalIgnoreCase);
    private static double? Find(List<Reading> sensors, SensorType type, double max, params string[] names)
    {
        foreach (var name in names)
            foreach (var s in sensors)
                if (s.Type == type && Is(s.Name, name) && Range(s.Value, 0, max) is double v) return v;
        return null;
    }

    private static int? CoreNumber(string name)
    {
        var offset = name.IndexOf("Core #", StringComparison.OrdinalIgnoreCase);
        if (offset < 0) return null;
        var digits = name.AsSpan(offset + 6);
        int length = 0;
        while (length < digits.Length && char.IsAsciiDigit(digits[length])) length++;
        return int.TryParse(digits[..length], out var number) ? number : null;
    }

    private static CpuHw ReadCpu(string name, List<Reading> sensors)
    {
        var cpu = new CpuHw { Name = name };
        cpu.PackageTempC = Find(sensors, SensorType.Temperature, 150,
            "CPU Package", "Core (Tdie)", "CPU (Tctl/Tdie)", "Core (Tctl/Tdie)", "Core (Tctl)");
        cpu.PackageTempC ??= sensors.Where(s => s.Type == SensorType.Temperature &&
            (CoreNumber(s.Name).HasValue || s.Name.Contains("CCD", StringComparison.OrdinalIgnoreCase)))
            .Select(s => Range(s.Value, 0, 150)).Max();
        var loads = sensors.Where(s => s.Type == SensorType.Load && CoreNumber(s.Name).HasValue)
            .OrderBy(s => s.Index).ToList();
        var temperatures = new Dictionary<int, double?>();
        foreach (var s in sensors)
        {
            if (s.Type != SensorType.Temperature || CoreNumber(s.Name) is not int core) continue;
            var value = Range(s.Value, 0, 150);
            if (!temperatures.TryGetValue(core, out var previous) || previous == null || value > previous)
                temperatures[core] = value;
        }
        cpu.TotalUsage = Find(sensors, SensorType.Load, 100, "CPU Total", "Total");
        foreach (var s in loads)
        {
            cpu.PerCoreUsage.Add(Range(s.Value, 0, 100));
            var number = CoreNumber(s.Name);
            cpu.PerCoreTempsC.Add(temperatures.GetValueOrDefault(number!.Value));
        }
        if (loads.Count == 0)
            cpu.PerCoreTempsC = sensors.Where(s => s.Type == SensorType.Temperature && CoreNumber(s.Name).HasValue)
                .OrderBy(s => s.Index).Select(s => Range(s.Value, 0, 150)).ToList();
        cpu.FrequencyMhz = sensors.Where(s => s.Type == SensorType.Clock && CoreNumber(s.Name).HasValue)
            .Select(s => Range(s.Value, 1, 20000)).Average();
        cpu.PowerW = Find(sensors, SensorType.Power, 10000, "CPU Package", "Package");
        return cpu;
    }

    private static CpuHw MergeCpu(CpuHw a, CpuHw b)
    {
        var ac = Math.Max(1, a.PerCoreUsage.Count);
        var bc = Math.Max(1, b.PerCoreUsage.Count);
        a.Name += " + " + b.Name;
        a.PackageTempC = a.PackageTempC.HasValue && b.PackageTempC.HasValue ? Math.Max(a.PackageTempC.Value, b.PackageTempC.Value) : null;
        a.TotalUsage = (a.TotalUsage * ac + b.TotalUsage * bc) / (ac + bc);
        a.FrequencyMhz = (a.FrequencyMhz * ac + b.FrequencyMhz * bc) / (ac + bc);
        a.PowerW += b.PowerW;
        a.PerCoreUsage.AddRange(b.PerCoreUsage);
        a.PerCoreTempsC.AddRange(b.PerCoreTempsC);
        return a;
    }

    private static GpuHw ReadGpu(IHardware hw, List<Reading> s, List<FanHw> fans, int index) => new() {
        Index = index, Name = hw.Name,
        Vendor = hw.HardwareType == HardwareType.GpuNvidia ? "nvidia" : hw.HardwareType == HardwareType.GpuAmd ? "amd" : "intel",
        UsagePercent = Find(s, SensorType.Load, 100, "GPU Core", "D3D 3D"),
        MemUsedMb = Find(s, SensorType.SmallData, 1048576, "GPU Memory Used", "D3D Dedicated Memory Used"),
        MemTotalMb = Find(s, SensorType.SmallData, 1048576, "GPU Memory Total"),
        TempC = Find(s, SensorType.Temperature, 150, "GPU Core", "GPU Temperature"),
        PowerW = Find(s, SensorType.Power, 10000, "GPU Package", "GPU Power", "GPU Core"),
        FanRpm = fans.FirstOrDefault()?.Rpm,
        FanPwm = fans.FirstOrDefault()?.PwmPercent,
    };

    private static MemoryHw? ReadMemory(List<Reading> s, WmiCache wmi)
    {
        var used = Find(s, SensorType.Data, 1048576, "Memory Used");
        var available = Find(s, SensorType.Data, 1048576, "Memory Available");
        if (!used.HasValue || !available.HasValue || used + available <= 0) return null;
        const double gib = 1024d * 1024 * 1024;
        var usedBytes = (ulong)(used.Value * gib);
        var totalBytes = (ulong)((used.Value + available.Value) * gib);
        if (totalBytes == 0) return null;
        return new MemoryHw {
            UsedBytes = usedBytes, TotalBytes = totalBytes, UsedPercent = 100d * usedBytes / totalBytes,
            Modules = wmi.MemoryModules, FrequencyMhz = wmi.MemoryFrequencyMhz, Channels = wmi.MemoryChannels,
        };
    }

    private static DiskHw ReadDisk(IHardware hw, List<Reading> s, WmiCache wmi, int fallbackIndex)
    {
        // AbstractStorage.Index is the Windows physical drive number (LHM 0.9.4).
        var storage = hw as AbstractStorage;
        var disk = new DiskHw { Index = storage?.Index ?? fallbackIndex, Model = hw.Name, Identifier = hw.Identifier.ToString() };
        disk.TempC = s.Where(r => r.Type == SensorType.Temperature).Select(r => Range(r.Value, 0, 150)).Max();
        disk.ReadBytesPerSec = CorrectStorageRate(Find(s, SensorType.Throughput, double.MaxValue, "Read Rate"), Stopwatch.Frequency);
        disk.WriteBytesPerSec = CorrectStorageRate(Find(s, SensorType.Throughput, double.MaxValue, "Write Rate"), Stopwatch.Frequency);
        var life = Find(s, SensorType.Level, 100, "Remaining Life");
        if (life.HasValue) disk.Health = life >= 50 ? "good" : life >= 20 ? "warning" : "critical";
        ApplyDiskMetadata(disk, wmi, storage?.Index);
        if (storage != null) disk.UsedBytes = wmi.GetUsedSpace(disk.Identifier, storage.DriveInfos);
        if (disk.UsedBytes > disk.TotalBytes && disk.TotalBytes > 0) disk.UsedBytes = null;
        return disk;
    }

    // LHM 0.9.4 uses TimeSpan.FromTicks(Stopwatch ticks) in AbstractStorage.
    // Revisit this correction when upgrading the pinned LHM dependency.
    public static double? CorrectStorageRate(double? rate, long frequency) =>
        frequency <= 0 ? null : Range(rate * (frequency / (double)TimeSpan.TicksPerSecond), 0, 1024d * 1024 * 1024 * 1024);

    public static void ApplyDiskMetadata(DiskHw disk, WmiCache wmi, int? physicalIndex)
    {
        var drive = physicalIndex.HasValue ? wmi.DiskDriveRows.Find(r => r.Index == physicalIndex) : UniqueModel(wmi.DiskDriveRows, disk.Model);
        if (drive != null)
        {
            disk.TotalBytes = drive.SizeBytes;
            if (drive.Bus.Length > 0) disk.Bus = drive.Bus;
        }
        // MSFT_PhysicalDisk.DeviceId is NOT a Win32 physical drive number.
        var storage = drive?.Serial.Length > 0
            ? Unique(wmi.StorageDiskRows, r => Is(r.Serial, drive.Serial))
            : UniqueModel(wmi.DiskDriveRows, disk.Model) != null ? UniqueModel(wmi.StorageDiskRows, disk.Model) : null;
        if (storage == null) return;
        if (storage.Bus.Length > 0) disk.Bus = storage.Bus;
        if (HealthRank(storage.Health) > HealthRank(disk.Health)) disk.Health = storage.Health;
    }

    private static int HealthRank(string health) => health switch { "critical" => 3, "warning" => 2, "good" => 1, _ => 0 };
    private static DiskMetadataRow? UniqueModel(List<DiskMetadataRow> rows, string model) =>
        model.Length == 0 ? null : Unique(rows, r => Is(r.Name, model));
    private static DiskMetadataRow? Unique(List<DiskMetadataRow> rows, Func<DiskMetadataRow, bool> predicate)
    {
        DiskMetadataRow? match = null;
        foreach (var row in rows)
        {
            if (!predicate(row)) continue;
            if (match != null) return null;
            match = row;
        }
        return match;
    }
}
