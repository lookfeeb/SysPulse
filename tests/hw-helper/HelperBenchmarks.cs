using System.Diagnostics;
using System.Reflection;
using System.Text.Json;
using LibreHardwareMonitor.Hardware;
using SysPulse.HwHelper;

// In-memory LHM sensors only. These timings exclude hardware/driver I/O and IPC.
public static class HelperBenchmarks
{
    private static long _sink;

    public static void Run()
    {
        var cpu = new BenchmarkHardware(HardwareType.Cpu);
        cpu.Add("CPU Package", SensorType.Temperature, 65);
        cpu.Add("CPU Total", SensorType.Load, 25);
        for (int core = 1; core <= 32; core++) {
            cpu.Add($"CPU Core #{core}", SensorType.Load, core);
            cpu.Add($"CPU Core #{core}", SensorType.Temperature, 45 + core);
            cpu.Add($"CPU Core #{core}", SensorType.Clock, 4200);
        }
        var memory = new BenchmarkHardware(HardwareType.Memory);
        memory.Add("Memory Used", SensorType.Data, 16);
        memory.Add("Memory Available", SensorType.Data, 16);
        var board = new BenchmarkHardware(HardwareType.SuperIO);
        for (int fan = 0; fan < 8; fan++) board.Add($"System Fan #{fan}", SensorType.Fan, 1200 + fan);
        IHardware[] devices = [cpu, memory, board];
        var metadata = new WmiCache();
        var snapshot = SnapshotBuilder.Build(devices, metadata);
        var mailbox = new SnapshotMailbox();
        mailbox.Publish(snapshot, Stopwatch.GetTimestamp());
        var results = new List<object> {
            Measure("mailbox-read", 100_000, () => mailbox.Read().AgeMs),
            Measure("build-108-sensors", 1_000, () => SnapshotBuilder.Build(devices, metadata).Cpu!.PerCoreUsage.Count),
        };
        using (var sampler = new HardwareSampler(devices, new object(), () => metadata)) {
            if (!SpinWait.SpinUntil(() => sampler.Read().Cpu != null, 5000)) throw new Exception("No publication");
            results.Add(Measure("snapshot-read", 100_000, () => sampler.Read().TimestampMs));
        }
        using var output = new MemoryStream();
        using var writer = new SnapshotResponseWriter(output);
        results.Add(Measure("snapshot-json-line", 1_000, () => {
            output.Position = 0;
            output.SetLength(0);
            writer.Write(1, snapshot);
            return output.Length;
        }));
        Console.WriteLine(JsonSerializer.Serialize(new { stopwatchFrequency = Stopwatch.Frequency, results }, new JsonSerializerOptions { WriteIndented = true }));
    }

    private static object Measure(string name, int iterations, Func<long> action)
    {
        for (int i = 0; i < Math.Min(iterations, 10_000); i++) _sink ^= action();
        GC.Collect(); GC.WaitForPendingFinalizers(); GC.Collect();
        var times = new double[21];
        var allocations = new double[21];
        for (int batch = 0; batch < times.Length; batch++) {
            long allocated = GC.GetAllocatedBytesForCurrentThread();
            long started = Stopwatch.GetTimestamp();
            long checksum = 0;
            for (int i = 0; i < iterations; i++) checksum ^= action();
            times[batch] = Stopwatch.GetElapsedTime(started).TotalNanoseconds / iterations;
            allocations[batch] = (GC.GetAllocatedBytesForCurrentThread() - allocated) / (double)iterations;
            Volatile.Write(ref _sink, checksum);
        }
        Array.Sort(times); Array.Sort(allocations);
        return new { name, iterationsPerBatch = iterations, batches = times.Length,
            medianNs = Math.Round(times[10], 1), p95BatchNs = Math.Round(times[19], 1), bytesPerOp = allocations[10] };
    }

    private sealed class BenchmarkHardware : Hardware
    {
        private static readonly Type SensorClass = typeof(Computer).Assembly.GetType("LibreHardwareMonitor.Hardware.Sensor")!;
        private static readonly PropertyInfo ValueProperty = SensorClass.GetProperty("Value")!;
        private readonly HardwareType _type;
        public BenchmarkHardware(HardwareType type) : base($"Synthetic {type}", new Identifier("benchmark", type.ToString()),
            Proxy.Create<ISettings>((method, args) => method.Name switch { "GetValue" => args![1], "Contains" => false, _ => null })) => _type = type;
        public override HardwareType HardwareType => _type;
        public override void Update() { }
        public void Add(string name, SensorType type, float value) {
            var sensor = (ISensor)Activator.CreateInstance(SensorClass, name, _active.Count, type, this, _settings)!;
            sensor.ValuesTimeWindow = TimeSpan.Zero;
            ValueProperty.SetValue(sensor, value);
            _active.Add(sensor);
        }
    }
}
