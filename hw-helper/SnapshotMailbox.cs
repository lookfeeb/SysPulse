using System.Diagnostics;
using LibreHardwareMonitor.Hardware;

namespace SysPulse.HwHelper;

/// Immutable publications: reading a snapshot never waits for a device query.
public sealed class SnapshotMailbox
{
    private sealed record Publication(HwSnapshot Snapshot, long Started);
    private Publication? _latest;

    public void Publish(HwSnapshot snapshot, long started) =>
        Interlocked.Exchange(ref _latest, new Publication(snapshot, started));

    public (HwSnapshot? Snapshot, long AgeMs) Read()
    {
        var publication = Volatile.Read(ref _latest);
        return publication is null ? (null, long.MaxValue)
            : (publication.Snapshot, (long)Stopwatch.GetElapsedTime(publication.Started).TotalMilliseconds);
    }
}

public sealed class HardwareSampler : IDisposable
{
    private readonly SnapshotMailbox _fast = new();
    private readonly SnapshotMailbox _storage = new();
    private readonly ManualResetEventSlim _stop = new(false);
    private readonly Thread _fastThread;
    private readonly Thread _storageThread;
    private readonly object _disposeGate = new();
    private bool _disposed;

    public HardwareSampler(ComputerHost host, Func<WmiCache> metadata)
        : this(host.Inner.Hardware.ToArray(), host, metadata) { }

    // The same collector path can run against synthetic devices without
    // opening drivers, allowing blocking I/O and shutdown to be verified.
    public HardwareSampler(IHardware[] roots, object sensorGate, Func<WmiCache> metadata)
    {
        // One LHM lifetime, disjoint device trees. Storage I/O never takes the
        // sensor/control gate and cannot delay the CPU/fan publication.
        var sensors = roots.Where(h => h.HardwareType != HardwareType.Storage).ToArray();
        var disks = roots.Where(h => h.HardwareType == HardwareType.Storage).ToArray();
        _fastThread = new Thread(() => Collect(sensorGate, sensors, metadata, _fast, 1000)) { IsBackground = true, Name = "sensors" };
        _storageThread = new Thread(() => Collect(new object(), disks, metadata, _storage, 3000)) { IsBackground = true, Name = "storage-sensors" };
        _fastThread.Start();
        _storageThread.Start();
    }

    private void Collect(object gate, IHardware[] devices, Func<WmiCache> metadata, SnapshotMailbox mailbox, int interval)
    {
        while (!_stop.IsSet) {
            var started = Stopwatch.GetTimestamp();
            try {
                HwSnapshot snapshot;
                lock (gate) snapshot = SnapshotBuilder.Build(ComputerHost.Refresh(devices), metadata());
                mailbox.Publish(snapshot, started);
                var elapsed = Stopwatch.GetElapsedTime(started).TotalMilliseconds;
                if (elapsed > 2000) Console.Error.WriteLine($"[hw-helper] {Thread.CurrentThread.Name} collection took {elapsed:F0} ms");
            } catch (Exception ex) { Console.Error.WriteLine($"[hw-helper] collector failed: {ex}"); }
            if (_stop.Wait(interval)) break;
        }
    }

    public HwSnapshot Read()
    {
        var (fast, age) = _fast.Read();
        var (storage, storageAge) = _storage.Read();
        return new HwSnapshot {
            TimestampMs = fast?.TimestampMs ?? 0,
            SampleAgeMs = age,
            StorageAgeMs = storageAge,
            Cpu = age <= 30000 ? fast?.Cpu : null,
            Gpus = age <= 30000 ? fast?.Gpus ?? new() : new(),
            Memory = age <= 30000 ? fast?.Memory : null,
            Motherboard = age <= 30000 ? fast?.Motherboard : null,
            Fans = age <= 30000 ? fast?.Fans ?? new() : new(),
            Disks = storageAge <= 30000 ? storage?.Disks ?? new() : new(),
        };
    }

    public void Dispose()
    {
        lock (_disposeGate) {
            if (_disposed) return;
            _stop.Set();
            // Driver calls cannot be safely aborted; the parent owns the
            // deadline. Never close LHM while a worker still uses a device.
            _fastThread.Join();
            _storageThread.Join();
            _stop.Dispose();
            _disposed = true;
        }
    }
}
