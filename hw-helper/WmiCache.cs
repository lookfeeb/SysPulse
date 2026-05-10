using System.Management;

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
    /// Perform all WMI queries. Safe to call from any thread; results are
    /// replaced atomically (list reference swap).
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
                "SELECT DeviceLocator, Capacity, Speed, Manufacturer, PartNumber FROM Win32_PhysicalMemory");
            foreach (ManagementObject m in searcher.Get())
            {
                modules.Add(new MemoryModule
                {
                    Slot = (m["DeviceLocator"]?.ToString() ?? "").Trim(),
                    CapacityBytes = m["Capacity"] is ulong cap ? cap : 0,
                    SpeedMtps = m["Speed"] is uint sp ? sp : null,
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
                    freqMhz = speeds[speeds.Count / 2];
                channels = modules.Count;
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
                "SELECT Model, Caption, InterfaceType, PNPDeviceID, MediaType FROM Win32_DiskDrive");
            return searcher
                .Get()
                .Cast<ManagementObject>()
                .Select(row => new
                {
                    Name = FirstText(row["Model"], row["Caption"]),
                    Bus = MapDiskInterface(row["InterfaceType"], row["PNPDeviceID"], row["MediaType"]),
                })
                .Where(row => !string.IsNullOrWhiteSpace(row.Name)
                           || !string.IsNullOrWhiteSpace(row.Bus))
                .Select(row => new DiskMetadataRow { Name = row.Name, Bus = row.Bus })
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
                "SELECT FriendlyName, Model, BusType, HealthStatus, OperationalStatus FROM MSFT_PhysicalDisk");
            return searcher
                .Get()
                .Cast<ManagementObject>()
                .Select(row => new DiskMetadataRow
                {
                    Name = FirstText(row["FriendlyName"], row["Model"]),
                    Bus = MapDiskBus(row["BusType"]),
                    Health = MapDiskHealth(row["HealthStatus"], row["OperationalStatus"]),
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

    private static string MapDiskHealth(object? healthStatus, object? operationalStatus)
    {
        var health = ToUInt16(healthStatus);
        if (health.HasValue)
        {
            return health.Value switch
            {
                0 => "good",
                1 => "warning",
                2 => "critical",
                5 => "unknown",
                _ => "unknown",
            };
        }

        var op = ToUInt16Array(operationalStatus);
        if (op.Any(value => value is 3 or 4 or 5 or 9 or 10 or 11 or 12 or 13 or 14))
            return "critical";
        if (op.Any(value => value is 2 or 6 or 7 or 8))
            return "warning";
        return op.Any(value => value == 1) ? "good" : "";
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
    public string Name { get; init; } = "";
    public string Bus { get; init; } = "";
    public string Health { get; init; } = "";
}
