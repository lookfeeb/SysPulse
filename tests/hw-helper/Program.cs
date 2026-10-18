using System.Reflection;
using System.Text.Json;
using LibreHardwareMonitor.Hardware;
using SysPulse.HwHelper;

// No driver, WMI query, real device, or fan write is used by this regression runner.
int passed = 0;
void Test(string name, Action action) { action(); passed++; Console.WriteLine($"PASS {name}"); }
void Check(bool condition) { if (!condition) throw new Exception("Assertion failed"); }

ISensor Sensor(string name, SensorType type, float? value, int index = 0, IControl? control = null) =>
    Proxy.Create<ISensor>((method, _) => method.Name switch {
        "get_Name" => name, "get_SensorType" => type, "get_Index" => index,
        "get_Identifier" => new Identifier("test", name.Replace(" ", "_"), type.ToString(), index.ToString()),
        "get_Value" => value, "get_Control" => control, _ => null,
    });

IHardware Device(HardwareType type, ISensor[] sensors, IHardware[]? children = null, bool fails = false, Action? updated = null) =>
    Proxy.Create<IHardware>((method, _) => {
        if (method.Name == "Update") { updated?.Invoke(); if (fails) throw new IOException("synthetic device failure"); return null; }
        return method.Name switch {
            "get_Name" => "Test device", "get_HardwareType" => type,
            "get_Identifier" => new Identifier("test", type.ToString()),
            "get_Sensors" => sensors, "get_SubHardware" => children ?? [], _ => null,
        };
    });
HwSnapshot Build(params IHardware[] devices) => SnapshotBuilder.Build(devices, new WmiCache());

Test("finite ranges", () => {
    foreach (var value in new double?[] { null, double.NaN, double.PositiveInfinity, -1, 151 })
        Check(SnapshotBuilder.Range(value, 0, 150) == null);
    Check(SnapshotBuilder.Range(0, 0, 150) == 0 && SnapshotBuilder.Range(150, 0, 150) == 150);
});
Test("request protocol bounds and incomplete frames", () => {
    using var input = new StringReader("one\ntwo\n");
    Check(ProtocolReader.ReadLine(input, 3) == "one");
    Check(ProtocolReader.ReadLine(input, 3) == "two" && ProtocolReader.ReadLine(input, 3) == null);
    foreach (var text in new[] { "large\n", "bad" })
    {
        bool rejected = false;
        try { ProtocolReader.ReadLine(new StringReader(text), 3); } catch (IOException) { rejected = true; }
        Check(rejected);
    }
});
Test("package priority and core alignment", () => {
    var cpu = Build(Device(HardwareType.Cpu, [
        Sensor("Core (CCD1)", SensorType.Temperature, 50), Sensor("CPU Package", SensorType.Temperature, 70),
        Sensor("CPU Core #2", SensorType.Temperature, 65, 1), Sensor("CPU Core #1", SensorType.Temperature, 55, 2),
        Sensor("CPU Core #1", SensorType.Load, 20, 1), Sensor("CPU Core #2", SensorType.Load, 40, 2),
        Sensor("CPU Total", SensorType.Load, 30), Sensor("CPU Core #1", SensorType.Clock, 3000),
        Sensor("CPU Core #2", SensorType.Clock, 4000, 1),
    ])).Cpu!;
    Check(cpu.PackageTempC == 70 && cpu.PerCoreTempsC.SequenceEqual(new double?[] { 55, 65 }));
    Check(cpu.FrequencyMhz == 3500 && cpu.TotalUsage == 30);
});
Test("missing load stays unknown and serializes", () => {
    var cpu = Build(Device(HardwareType.Cpu, [Sensor("CPU Total", SensorType.Load, float.NaN), Sensor("CPU Core #1", SensorType.Load, null)])).Cpu!;
    Check(cpu.TotalUsage == null && cpu.PerCoreUsage.Single() == null);
    JsonSerializer.Serialize(cpu);
});
Test("sensor exception and single capture", () => {
    int reads = 0;
    var bad = Probe.Sensor("CPU Package", SensorType.Temperature, () => throw new IOException("synthetic"));
    var good = Probe.Sensor("CPU Core #1", SensorType.Temperature, () => { reads++; return 60; });
    var snap = Build(Device(HardwareType.Cpu, [bad, good]));
    Check(snap.Cpu?.PackageTempC == 60 && reads == 1);
});
Test("library history is disabled without affecting live values", () => {
    var window = TimeSpan.FromDays(1);
    var sensor = Proxy.Create<ISensor>((method, args) => {
        if (method.Name == "set_ValuesTimeWindow") { window = (TimeSpan)args![0]!; return null; }
        return method.Name switch {
            "get_ValuesTimeWindow" => window, "get_Name" => "CPU Package",
            "get_Value" => 62f, "get_SensorType" => SensorType.Temperature,
            "get_Index" => 0, "get_Identifier" => new Identifier("history", "test"), _ => null,
        };
    });
    Check(Build(Device(HardwareType.Cpu, [sensor])).Cpu?.PackageTempC == 62 && window == TimeSpan.Zero);
});
Test("failed parent and recursive children", () => {
    int updates = 0;
    int failures = 0;
    var leaf = Device(HardwareType.SuperIO, [Sensor("Fan", SensorType.Fan, 900)], updated: () => updates++);
    var parent = Device(HardwareType.Motherboard, [], [Device(HardwareType.SuperIO, [], [leaf])], fails: true, updated: () => failures++);
    var fresh = ComputerHost.Refresh([parent, leaf]);
    Check(fresh.Count == 2 && updates == 1 && !fresh.Contains(parent));
    Check(SnapshotBuilder.Build(fresh, new WmiCache()).Fans.Single().Rpm == 900);
    ComputerHost.Refresh([parent]);
    Check(failures == 1 && updates == 2);
});
Test("physical and virtual memory separation", () => {
    var memory = Build(Device(HardwareType.Memory, [
        Sensor("Virtual Memory Used", SensorType.Data, 100), Sensor("Virtual Memory Available", SensorType.Data, 200),
        Sensor("Memory Used", SensorType.Data, 8), Sensor("Memory Available", SensorType.Data, 24),
    ])).Memory!;
    Check(memory.TotalBytes == 32UL * 1024 * 1024 * 1024 && memory.UsedPercent == 25 && memory.Channels == null);
    Check(Build(Device(HardwareType.Memory, [Sensor("Virtual Memory Used", SensorType.Data, 1)])).Memory == null);
});
Test("configured memory speed", () => {
    Check(WmiCache.ConfiguredSpeed(4800u, 5600u) == 4800);
    Check(WmiCache.ConfiguredSpeed(0u, 5600u) == 5600);
    Check(WmiCache.ConfiguredSpeed(0u, 0u) == null);
});
Test("duplicate disk models use physical identity", () => {
    var wmi = new WmiCache();
    wmi.DiskDriveRows.AddRange([
        new() { Index = 0, Name = "Same", Serial = "A", SizeBytes = 100 },
        new() { Index = 2, Name = "Same", Serial = "B", SizeBytes = 200 },
    ]);
    wmi.StorageDiskRows.AddRange([
        new() { Name = "Same", Serial = "B", Bus = "nvme", Health = "critical" },
        new() { Name = "Same", Serial = "A", Bus = "sata", Health = "good" },
    ]);
    var disk = new DiskHw { Model = "Same" };
    SnapshotBuilder.ApplyDiskMetadata(disk, wmi, 2);
    Check(disk.TotalBytes == 200 && disk.Bus == "nvme" && disk.Health == "critical");
    var ambiguous = new DiskHw { Model = "Same" };
    SnapshotBuilder.ApplyDiskMetadata(ambiguous, wmi, null);
    Check(ambiguous.TotalBytes == 0 && ambiguous.Health == "unknown");
    var missing = new DiskHw { Model = "different", Index = 0 };
    SnapshotBuilder.ApplyDiskMetadata(missing, wmi, null);
    Check(missing.TotalBytes == 0);
});
Test("stopwatch frequency conversion", () => {
    Check(SnapshotBuilder.CorrectStorageRate(100, 10_000_000) == 100);
    Check(SnapshotBuilder.CorrectStorageRate(100, 1_000_000_000) == 10000);
    Check(SnapshotBuilder.CorrectStorageRate(double.PositiveInfinity, 10_000_000) == null);
    Check(SnapshotBuilder.CorrectStorageRate(100, 0) == null);
});
Test("WMI operational status semantics", () => {
    Check(WmiCache.MapDiskHealth(null, new ushort[] { 2 }) == "good");
    Check(WmiCache.MapDiskHealth(null, new ushort[] { 6 }) == "critical");
    Check(WmiCache.MapDiskHealth(null, new ushort[] { 3 }) == "warning");
    Check(WmiCache.MapDiskHealth(0, new ushort[] { 6 }) == "critical");
});
Test("multiple CPU aggregation", () => {
    var snap = Build(
        Device(HardwareType.Cpu, [Sensor("CPU Total", SensorType.Load, 20), Sensor("CPU Package", SensorType.Temperature, 50)]),
        Device(HardwareType.Cpu, [Sensor("CPU Total", SensorType.Load, 80), Sensor("CPU Package", SensorType.Temperature, 70)]));
    Check(snap.Cpu?.TotalUsage == 50 && snap.Cpu.PackageTempC == 70);
});
Test("RPM preserves stopped and high-speed fans without accepting invalid values", () => {
    foreach (var rpm in new float?[] { 0, 12000, 36000 })
        Check(Build(Device(HardwareType.SuperIO, [Sensor("Fan", SensorType.Fan, rpm)])).Fans.Single().Rpm == rpm);
    foreach (var rpm in new float?[] { null, -1, float.NaN, float.PositiveInfinity })
        Check(Build(Device(HardwareType.SuperIO, [Sensor("Fan", SensorType.Fan, rpm)])).Fans.Single().Rpm == null);
});
Test("board label matches RPM 0 to PWM 2; telemetry and writes share identity", () => {
    var cpuControl = Proxy.Create<IControl>((_, _) => null);
    var wrongControl = Proxy.Create<IControl>((_, _) => null);
    var rpm = Sensor("CPU Fan", SensorType.Fan, 1200, 0);
    var hw = Device(HardwareType.SuperIO, [rpm,
        Sensor("System Fan", SensorType.Control, 90, 0, wrongControl),
        Sensor("CPU Fan", SensorType.Control, 37, 2, cpuControl)]);
    var fan = Build(hw).Fans.Single();
    Check(fan.PwmPercent == 37 && fan.Controllable);
    Check(ReferenceEquals(FanControl.FindControl([hw], rpm.Identifier.ToString()), cpuControl));
});
Test("same index, generic labels, ambiguous names and direct control IDs cannot guess a mapping", () => {
    var control = Proxy.Create<IControl>((_, _) => null);
    var rpm = Sensor("CPU Fan", SensorType.Fan, 1000);
    var wrong = Sensor("System Fan", SensorType.Control, 80, 0, control);
    var wrongHw = Device(HardwareType.SuperIO, [rpm, wrong]);
    Check(!Build(wrongHw).Fans.Single().Controllable && Build(wrongHw).Fans.Single().PwmPercent == null);
    Check(FanControl.FindControl([wrongHw], rpm.Identifier.ToString()) == null);
    Check(FanControl.FindControl([wrongHw], wrong.Identifier.ToString()) == null);
    var generic = Device(HardwareType.SuperIO, [Sensor("Fan #1", SensorType.Fan, 1000), Sensor("Fan #1", SensorType.Control, 50, 0, control)]);
    Check(!Build(generic).Fans.Single().Controllable);
    var ambiguous = Device(HardwareType.SuperIO, [rpm,
        Sensor("CPU Fan", SensorType.Control, 40, 1, control), Sensor("CPU Fan", SensorType.Control, 60, 2, control)]);
    Check(!Build(ambiguous).Fans.Single().Controllable);
    var duplicateFans = Device(HardwareType.SuperIO, [rpm, Sensor("CPU Fan", SensorType.Fan, 900, 1), Sensor("CPU Fan", SensorType.Control, 40, 2, control)]);
    Check(Build(duplicateFans).Fans.All(f => !f.Controllable));
});
Test("explicit control attachment is accepted without index or label guessing", () => {
    var control = Proxy.Create<IControl>((_, _) => null);
    var rpm = Sensor("Fan", SensorType.Fan, 800, 0, control);
    var hw = Device(HardwareType.SuperIO, [rpm, Sensor("PWM", SensorType.Control, 44, 3, control)]);
    var fan = Build(hw).Fans.Single();
    Check(fan.Controllable && fan.PwmPercent == 44);
    Check(ReferenceEquals(FanControl.FindControl([hw], rpm.Identifier.ToString()), control));
});
Test("NVIDIA success then silent driver failure publishes null, including GPU summary", () => {
    var hw = new SyntheticNvidiaHardware();
    var rpm = hw.Add("GPU Fan", SensorType.Fan, 0);
    var pwm = hw.Add("GPU Fan", SensorType.Control, 0);
    hw.OnUpdate = () => { hw.Write(rpm, 18000); hw.Write(pwm, 40); };
    var first = SnapshotBuilder.Build(ComputerHost.Refresh([hw]), new WmiCache());
    Check(first.Fans.Single().Rpm == 18000 && first.Gpus.Single().FanRpm == 18000);
    Check(FanChannels.Capture(hw).Single(s => s.Type == SensorType.Control).Value == 40);
    hw.OnUpdate = () => { }; // Models NVAPI returning an error without throwing or changing Value.
    var failed = SnapshotBuilder.Build(ComputerHost.Refresh([hw]), new WmiCache());
    Check(failed.Fans.Single().Rpm == null && failed.Gpus.Single().FanRpm == null);
    Check(FanChannels.Capture(hw).Single(s => s.Type == SensorType.Control).Value == null);
});
Test("NVIDIA partial reads, zero RPM, identical readings and recovery remain distinct", () => {
    var hw = new SyntheticNvidiaHardware();
    var rpm = hw.Add("GPU Fan", SensorType.Fan, 0);
    var pwm = hw.Add("GPU Fan", SensorType.Control, 0);
    hw.OnUpdate = () => { hw.Write(rpm, 900); hw.Write(pwm, 40); };
    for (var i = 0; i < 3; i++) Check(SnapshotBuilder.Build(ComputerHost.Refresh([hw]), new WmiCache()).Fans.Single().Rpm == 900);
    hw.OnUpdate = () => hw.Write(rpm, 0);
    Check(SnapshotBuilder.Build(ComputerHost.Refresh([hw]), new WmiCache()).Fans.Single().Rpm == 0);
    Check(FanChannels.Capture(hw).Single(s => s.Type == SensorType.Control).Value == null);
    hw.OnUpdate = () => hw.Write(pwm, 55);
    Check(SnapshotBuilder.Build(ComputerHost.Refresh([hw]), new WmiCache()).Fans.Single().Rpm == null);
    Check(FanChannels.Capture(hw).Single(s => s.Type == SensorType.Control).Value == 55);
    hw.OnUpdate = () => hw.Write(rpm, 1400);
    Check(SnapshotBuilder.Build(ComputerHost.Refresh([hw]), new WmiCache()).Fans.Single().Rpm == 1400);
});
Test("unsupported NVIDIA cache invalidation fails closed without dropping other sensors", () => {
    var hw = Device(HardwareType.GpuNvidia, [Sensor("GPU Fan", SensorType.Fan, 1200), Sensor("GPU Core", SensorType.Temperature, 50)]);
    var snap = SnapshotBuilder.Build(ComputerHost.Refresh([hw]), new WmiCache());
    Check(snap.Fans.Single().Rpm == null && snap.Gpus.Single().TempC == 50);
});
Console.WriteLine($"{passed} hardware regressions passed.");

public class Proxy : DispatchProxy
{
    public Func<MethodInfo, object?[]?, object?> Handler = null!;
    protected override object? Invoke(MethodInfo? method, object?[]? args) => Handler(method!, args);
    public static T Create<T>(Func<MethodInfo, object?[]?, object?> handler) where T : class
    {
        var proxy = Create<T, Proxy>();
        ((Proxy)(object)proxy).Handler = handler;
        return proxy;
    }
}
public static class Probe
{
    public static ISensor Sensor(string name, SensorType type, Func<float?> read) => Proxy.Create<ISensor>((method, _) => method.Name switch {
        "get_Name" => name, "get_SensorType" => type, "get_Index" => 0,
        "get_Identifier" => new Identifier("test", name.Replace(" ", "_")),
        "get_Value" => read(), "get_Control" => null, _ => null,
    });
}

// Real LHM in-memory Sensor instances attached to a synthetic Hardware object.
// Reflection is restricted to the pinned library's cache; no driver is opened.
public sealed class SyntheticNvidiaHardware : Hardware
{
    private static readonly Type SensorClass = typeof(Computer).Assembly.GetType("LibreHardwareMonitor.Hardware.Sensor")!;
    private static readonly PropertyInfo ValueProperty = SensorClass.GetProperty("Value")!;
    public SyntheticNvidiaHardware() : base("Synthetic NVIDIA", new Identifier("synthetic", "gpu"),
        Proxy.Create<ISettings>((method, args) => method.Name switch {
            "GetValue" => args![1], "Contains" => false, _ => null,
        })) { }
    public override HardwareType HardwareType => HardwareType.GpuNvidia;
    public Action? OnUpdate { get; set; }
    public override void Update() => OnUpdate?.Invoke();
    public ISensor Add(string name, SensorType type, int index)
    {
        var sensor = (ISensor)Activator.CreateInstance(SensorClass, name, index, type, this, _settings)!;
        _active.Add(sensor);
        return sensor;
    }
    public void Write(ISensor sensor, float? value) => ValueProperty.SetValue(sensor, value);
}
