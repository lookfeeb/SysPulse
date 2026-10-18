using System.Management;
using System.Diagnostics;

namespace SysPulse.HwHelper;

/// <summary>
/// Caches WMI query results that rarely change (memory modules, disk metadata).
/// Populated once at startup; the snapshot hot path never touches WMI.
/// </summary>
public sealed class WmiCache
{
    public List<MemoryModule> MemoryModules { get; private set; } = new();
    public double? MemoryFrequencyMhz { get; private set; }
    public int? MemoryChannels { get; private set; }
    public List<DiskMetadataRow> DiskDriveRows { get; private set; } = new();
    public List<DiskMetadataRow> StorageDiskRows { get; private set; } = new();

    /// <summary>
    /// Populate an unpublished cache; publish it only after this method returns.
    /// </summary>
    public void Refresh()
    {
        MemoryModules = QueryMemoryModules(out var freqMhz, out var channels);
        MemoryFrequencyMhz = freqMhz;
        MemoryChannels = channels;
        DiskDriveRows = QueryDiskDriveMetadata();
        StorageDiskRows = QueryStorageDiskMetadata();
    }

    // -----------------------------------------------------------------------
    // Memory modules via Win32_PhysicalMemory
    // -----------------------------------------------------------------------

    private static List<MemoryModule> QueryMemoryModules(out double? freqMhz, out int? channels)
    {
        freqMhz = null;
        channels = null;
        var modules = new List<MemoryModule>();

        try
        {
            using var searcher = new ManagementObjectSearcher(
                "SELECT DeviceLocator, Capacity, Speed, ConfiguredClockSpeed, Manufacturer, PartNumber FROM Win32_PhysicalMemory");
            searcher.Options.Timeout = TimeSpan.FromSeconds(2);
            using var results = searcher.Get();
            foreach (ManagementObject m in results)
            {
                using var row = m;
                modules.Add(new MemoryModule
                {
                    Slot = (m["DeviceLocator"]?.ToString() ?? "").Trim(),
                    CapacityBytes = m["Capacity"] is ulong cap ? cap : 0,
                    SpeedMtps = ConfiguredSpeed(m["ConfiguredClockSpeed"], m["Speed"]),
                    Manufacturer = (m["Manufacturer"]?.ToString() ?? "").Trim(),
                    PartNumber = (m["PartNumber"]?.ToString() ?? "").Trim(),
                });
            }

            if (modules.Count > 0)
            {
                var speeds = modules
                    .Select(x => x.SpeedMtps)
                    .Where(x => x is > 0)
                    .Select(x => x!.Value)
                    .OrderBy(x => x)
                    .ToList();
                if (speeds.Count > 0)
                    freqMhz = speeds.All(s => s == speeds[0]) ? speeds[0] : null;
                // DIMM count does not identify active memory channels.
            }
        }
        catch { /* WMI optional */ }

        return modules;
    }

    // -----------------------------------------------------------------------
    // Disk metadata via Win32_DiskDrive + MSFT_PhysicalDisk
    // -----------------------------------------------------------------------

    private static List<DiskMetadataRow> QueryDiskDriveMetadata()
    {
        try
        {
            using var searcher = new ManagementObjectSearcher(
                "SELECT Index, Size, SerialNumber, Model, Caption, InterfaceType, PNPDeviceID, MediaType FROM Win32_DiskDrive");
            searcher.Options.Timeout = TimeSpan.FromSeconds(2);
            using var results = searcher.Get();
            return results
                .Cast<ManagementObject>()
                .Select(row =>
                {
                    using (row) return new DiskMetadataRow {
                        Index = row["Index"] is uint index && index <= int.MaxValue ? (int)index : null,
                        SizeBytes = row["Size"] is ulong size ? size : 0,
                        Serial = FirstText(row["SerialNumber"]),
                        Name = FirstText(row["Model"], row["Caption"]),
                        Bus = MapDiskInterface(row["InterfaceType"], row["PNPDeviceID"], row["MediaType"]),
                    };
                })
                .Where(row => !string.IsNullOrWhiteSpace(row.Name)
                           || !string.IsNullOrWhiteSpace(row.Bus))
                .ToList();
        }
        catch
        {
            return new List<DiskMetadataRow>();
        }
    }

    private static List<DiskMetadataRow> QueryStorageDiskMetadata()
    {
        try
        {
            using var searcher = new ManagementObjectSearcher(
                @"root\Microsoft\Windows\Storage",
                "SELECT FriendlyName, Model, SerialNumber, BusType, HealthStatus, OperationalStatus FROM MSFT_PhysicalDisk");
            searcher.Options.Timeout = TimeSpan.FromSeconds(2);
            using var results = searcher.Get();
            return results
                .Cast<ManagementObject>()
                .Select(row =>
                {
                    using (row) return new DiskMetadataRow {
                        Name = FirstText(row["Model"], row["FriendlyName"]),
                        Serial = FirstText(row["SerialNumber"]),
                        Bus = MapDiskBus(row["BusType"]),
                        Health = MapDiskHealth(row["HealthStatus"], row["OperationalStatus"]),
                    };
                })
                .Where(row => !string.IsNullOrWhiteSpace(row.Name)
                           || !string.IsNullOrWhiteSpace(row.Bus)
                           || !string.IsNullOrWhiteSpace(row.Health))
                .ToList();
        }
        catch
        {
            return new List<DiskMetadataRow>();
        }
    }

    // -----------------------------------------------------------------------
    // Helpers (moved from SnapshotBuilder so both can share)
    // -----------------------------------------------------------------------

    public static double? ConfiguredSpeed(object? configured, object? rated) =>
        configured is uint active && active > 0 ? active : rated is uint speed && speed > 0 ? speed : null;

    private readonly Dictionary<string, (long Timestamp, ulong? Used)> _space = new();

    public ulong? GetUsedSpace(string id, DriveInfo[] volumes)
    {
        if (_space.TryGetValue(id, out var cached) && Stopwatch.GetElapsedTime(cached.Timestamp).TotalSeconds < 30)
            return cached.Used;
        ulong? used = null;
        try
        {
            if (volumes.Length > 0)
            {
                ulong total = 0;
                foreach (var volume in volumes)
                {
                    if (!volume.IsReady) throw new IOException("Volume unavailable");
                    total = checked(total + (ulong)Math.Max(0, volume.TotalSize - volume.TotalFreeSpace));
                }
                used = total;
            }
        }
        catch { /* Incomplete volume accounting is unknown, not zero. */ }
        if (_space.Count >= 256) _space.Clear();
        _space[id] = (Stopwatch.GetTimestamp(), used);
        return used;
    }


    private static string FirstText(params object?[] values)
    {
        foreach (var value in values)
        {
            var text = value?.ToString()?.Trim();
            if (!string.IsNullOrWhiteSpace(text)) return text;
        }
        return "";
    }

    private static string MapDiskInterface(object? interfaceType, object? pnpDeviceId, object? mediaType)
    {
        var text = $"{interfaceType} {pnpDeviceId} {mediaType}".ToLowerInvariant();
        if (text.Contains("nvme")) return "nvme";
        if (text.Contains("usb")) return "usb";
        if (text.Contains("sata")) return "sata";
        if (text.Contains("raid")) return "raid";
        if (text.Contains("sas")) return "sas";
        if (text.Contains("scsi")) return "scsi";
        if (text.Contains("ide") || text.Contains("ata")) return "sata";
        return "";
    }

    private static string MapDiskBus(object? busType)
    {
        return ToUInt16(busType) switch
        {
            1 => "scsi",
            2 => "atapi",
            3 => "ata",
            4 => "1394",
            5 => "ssa",
            6 => "fc",
            7 => "usb",
            8 => "raid",
            9 => "iscsi",
            10 => "sas",
            11 => "sata",
            12 => "sd",
            13 => "mmc",
            15 => "file",
            16 => "spaces",
            17 => "nvme",
            18 => "scm",
            19 => "ufs",
            _ => "",
        };
    }

    public static string MapDiskHealth(object? healthStatus, object? operationalStatus)
    {
        var health = ToUInt16(healthStatus);
        var op = ToUInt16Array(operationalStatus);
        if (health == 2 || op.Any(value => value is 6 or 7 or 12 or 13 or 16))
            return "critical";
        if (health == 1 || op.Any(value => value is 3 or 4 or 5 or 9 or 10))
            return "warning";
        return health == 0 || op.Any(value => value == 2) ? "good" : "unknown";
    }

    private static ushort? ToUInt16(object? value)
    {
        try
        {
            return value == null ? null : Convert.ToUInt16(value);
        }
        catch
        {
            return null;
        }
    }

    private static IReadOnlyList<ushort> ToUInt16Array(object? value)
    {
        if (value is ushort[] items) return items;
        if (value is Array array)
        {
            var values = new List<ushort>();
            foreach (var item in array)
            {
                var number = ToUInt16(item);
                if (number.HasValue) values.Add(number.Value);
            }
            return values;
        }
        var single = ToUInt16(value);
        return single.HasValue ? new[] { single.Value } : Array.Empty<ushort>();
    }
}

/// <summary>
/// Shared DTO for disk metadata from WMI.
/// </summary>
public sealed class DiskMetadataRow
{
    public int? Index { get; init; }
    public ulong SizeBytes { get; init; }
    public string Serial { get; init; } = "";
    public string Name { get; init; } = "";
    public string Bus { get; init; } = "";
    public string Health { get; init; } = "";
}
