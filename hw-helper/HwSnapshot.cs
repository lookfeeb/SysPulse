using System.Text.Json.Serialization;

namespace SysPulse.HwHelper;

// ---------------------------------------------------------------------------
// HwSnapshot DTO. Mirrors src-tauri/src/hw/snapshot.rs.
// All numeric fields are nullable when the underlying sensor is unavailable.
// ---------------------------------------------------------------------------

public sealed class HwSnapshot
{
    public HwSnapshot()
    {
        Gpus = new(); Disks = new(); Fans = new();
    }

    // Reuse immutable publications without allocating lists just to overwrite
    // them. Missing/expired data still gets independent empty collections.
    internal HwSnapshot(HwSnapshot? fast, long age, HwSnapshot? storage, long storageAge)
    {
        TimestampMs = fast?.TimestampMs ?? 0;
        SampleAgeMs = age;
        StorageAgeMs = storageAge;
        if (age > 30000) fast = null;
        if (storageAge > 30000) storage = null;
        Cpu = fast?.Cpu;
        Gpus = fast?.Gpus ?? new();
        Memory = fast?.Memory;
        Motherboard = fast?.Motherboard;
        Fans = fast?.Fans ?? new();
        Disks = storage?.Disks ?? new();
    }

    [JsonPropertyName("timestampMs")] public long TimestampMs { get; set; }
    [JsonPropertyName("sampleAgeMs")] public long SampleAgeMs { get; set; }
    [JsonPropertyName("storageAgeMs")] public long StorageAgeMs { get; set; }
    [JsonPropertyName("cpu")] public CpuHw? Cpu { get; set; }
    [JsonPropertyName("gpus")] public List<GpuHw> Gpus { get; set; }
    [JsonPropertyName("memory")] public MemoryHw? Memory { get; set; }
    [JsonPropertyName("disks")] public List<DiskHw> Disks { get; set; }
    [JsonPropertyName("motherboard")] public MotherboardHw? Motherboard { get; set; }
    [JsonPropertyName("fans")] public List<FanHw> Fans { get; set; }
}

public sealed class CpuHw
{
    [JsonPropertyName("name")] public string Name { get; set; } = "";
    [JsonPropertyName("packageTempC")] public double? PackageTempC { get; set; }
    [JsonPropertyName("perCoreTempsC")] public List<double?> PerCoreTempsC { get; set; } = new();
    [JsonPropertyName("perCoreUsage")] public List<double?> PerCoreUsage { get; set; } = new();
    [JsonPropertyName("totalUsage")] public double? TotalUsage { get; set; }
    [JsonPropertyName("frequencyMhz")] public double? FrequencyMhz { get; set; }
    [JsonPropertyName("powerW")] public double? PowerW { get; set; }
}

public sealed class GpuHw
{
    [JsonPropertyName("index")] public int Index { get; set; }
    [JsonPropertyName("name")] public string Name { get; set; } = "";
    [JsonPropertyName("vendor")] public string Vendor { get; set; } = "unknown"; // nvidia | amd | intel | unknown
    [JsonPropertyName("usagePercent")] public double? UsagePercent { get; set; }
    [JsonPropertyName("memUsedMb")] public double? MemUsedMb { get; set; }
    [JsonPropertyName("memTotalMb")] public double? MemTotalMb { get; set; }
    [JsonPropertyName("tempC")] public double? TempC { get; set; }
    [JsonPropertyName("powerW")] public double? PowerW { get; set; }
    [JsonPropertyName("fanRpm")] public double? FanRpm { get; set; }
    [JsonPropertyName("fanPwm")] public double? FanPwm { get; set; }
}

public sealed class MemoryHw
{
    [JsonPropertyName("totalBytes")] public ulong TotalBytes { get; set; }
    [JsonPropertyName("usedBytes")] public ulong UsedBytes { get; set; }
    [JsonPropertyName("usedPercent")] public double UsedPercent { get; set; }
    [JsonPropertyName("swapTotalBytes")] public ulong SwapTotalBytes { get; set; }
    [JsonPropertyName("swapUsedBytes")] public ulong SwapUsedBytes { get; set; }
    [JsonPropertyName("modules")] public List<MemoryModule> Modules { get; set; } = new();
    [JsonPropertyName("frequencyMhz")] public double? FrequencyMhz { get; set; }
    [JsonPropertyName("channels")] public int? Channels { get; set; }
}

public sealed class MemoryModule
{
    [JsonPropertyName("slot")] public string Slot { get; set; } = "";
    [JsonPropertyName("capacityBytes")] public ulong CapacityBytes { get; set; }
    [JsonPropertyName("speedMtps")] public double? SpeedMtps { get; set; }
    [JsonPropertyName("manufacturer")] public string Manufacturer { get; set; } = "";
    [JsonPropertyName("partNumber")] public string PartNumber { get; set; } = "";
}

public sealed class DiskHw
{
    [JsonPropertyName("index")] public int Index { get; set; }
    [JsonPropertyName("model")] public string Model { get; set; } = "";
    [JsonPropertyName("bus")] public string Bus { get; set; } = "unknown"; // sata | nvme | usb | unknown
    [JsonPropertyName("tempC")] public double? TempC { get; set; }
    [JsonPropertyName("health")] public string Health { get; set; } = "unknown"; // good | warning | critical | unknown
    [JsonPropertyName("readBytesPerSec")] public double? ReadBytesPerSec { get; set; }
    [JsonPropertyName("writeBytesPerSec")] public double? WriteBytesPerSec { get; set; }
    [JsonPropertyName("totalBytes")] public ulong TotalBytes { get; set; }
    [JsonPropertyName("usedBytes")] public ulong? UsedBytes { get; set; }
    [JsonPropertyName("identifier")] public string Identifier { get; set; } = "";
}

public sealed class MotherboardHw
{
    [JsonPropertyName("interfaces")] public List<BoardInterface> Interfaces { get; set; } = new();
    [JsonPropertyName("interfacesAgeMs")] public long InterfacesAgeMs { get; set; } = long.MaxValue;
    [JsonPropertyName("vendor")] public string Vendor { get; set; } = "";
    [JsonPropertyName("model")] public string Model { get; set; } = "";
    [JsonPropertyName("superIo")] public string? SuperIo { get; set; }
    [JsonPropertyName("temperaturesC")] public List<NamedValue> TemperaturesC { get; set; } = new();
    [JsonPropertyName("voltagesV")] public List<NamedValue> VoltagesV { get; set; } = new();
}

public sealed class BoardInterface
{
    public UsbInterfaceDetails? Usb { get; set; }
    public string Id { get; set; } = "";
    public string Name { get; set; } = "";
    public string Kind { get; set; } = "";
    public string? Connector { get; set; }
    public string? SupportedSpeed { get; set; }
    public string? CurrentSpeed { get; set; }
    public string Status { get; set; } = "unknown";
    public string? Device { get; set; }
    public string Source { get; set; } = "";
}

public sealed class UsbInterfaceDetails
{
    public int LogicalPortCount { get; set; } = 1;
    public bool? UserConnectable { get; set; }
    public string? MaximumSpeedSource { get; set; }
    public string? DeviceCapability { get; set; }
    public string? DeviceCapabilitySource { get; set; }
    public string? TopologyNote { get; set; }
}

public sealed class NamedValue
{
    [JsonPropertyName("name")] public string Name { get; set; } = "";
    [JsonPropertyName("value")] public double Value { get; set; }
    [JsonPropertyName("identifier")] public string Identifier { get; set; } = "";
}

public sealed class FanHw
{
    [JsonPropertyName("id")] public string Id { get; set; } = "";              // LHM Identifier (stable)
    [JsonPropertyName("name")] public string Name { get; set; } = "";
    [JsonPropertyName("rpm")] public double? Rpm { get; set; }
    [JsonPropertyName("pwmPercent")] public double? PwmPercent { get; set; }
    [JsonPropertyName("controllable")] public bool Controllable { get; set; }
}
