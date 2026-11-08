using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;
namespace SysPulse.HwHelper;

/// Versioned unmanaged entry point loaded with hostfxr. Called by one Rust worker.
/// Operations: 0 start, 1 snapshot, 2 manual fan, 3 reset fan, 4 reset all, 5 close.
/// Result: 0 success, 1 output buffer too small, negative = UTF-8 error in output.
/// All managed exceptions are contained here; native/driver faults remain process-wide.
public static unsafe class InProcessBridge
{
    private static readonly object Gate = new();
    private static ComputerHost? _host;
    private static HardwareSampler? _sampler;
    private static BoardInterfaceCollector? _ports;
    private static SnapshotBinary? _binary;

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    public static int Call(int version, int op, byte* name, int nameLength, double value,
        byte* output, int capacity, int* written)
    {
        if (written == null || capacity < 0 || capacity > 4 * 1024 * 1024 ||
            (capacity > 0 && output == null) || nameLength < 0 || nameLength > 4096 ||
            (nameLength > 0 && name == null)) return -1;
        *written = 0;
        // Reject version before opening hardware or performing a control write.
        if (version != 1) return Error("ABI_VERSION: expected 1", output, capacity, written);
        lock (Gate)
        {
            try
            {
                if (op == 0) {
                    if (_host != null) return 0;
                    var host = new ComputerHost();
                    try {
                        host.Open();
                        var empty = new WmiCache();
                        var metadata = Task.Run(() => { var m = new WmiCache(); m.Refresh(); return m; });
                        _ports = new BoardInterfaceCollector();
                        _sampler = new HardwareSampler(host, () => metadata.IsCompletedSuccessfully ? metadata.Result : empty);
                        _binary = new SnapshotBinary();
                        _host = host;
                        return 0;
                    } catch { _sampler?.Dispose(); _sampler = null; _ports?.Dispose(); _ports = null; host.Dispose(); throw; }
                }
                if (op == 5) {
                    // Stop collectors before releasing devices; never unload CLR under live workers.
                    _sampler?.Dispose(); _sampler = null;
                    _ports?.Dispose(); _ports = null;
                    _host?.Dispose(); _host = null;
                    _binary?.Dispose(); _binary = null;
                    return 0;
                }
                if (_host == null || _sampler == null) return Error("NOT_RUNNING: hardware module is closed", output, capacity, written);
                if (op == 1) {
                    var snapshot = _sampler.Read();
                    _ports?.Attach(snapshot);
                    var bytes = _binary!.Encode(snapshot);
                    *written = bytes.Length;
                    if (capacity < bytes.Length) return 1;
                    bytes.CopyTo(new Span<byte>(output, capacity));
                    return 0;
                }
                if (!Monitor.TryEnter(_host, 100)) return Error("BUSY: hardware update in progress", output, capacity, written);
                try {
                    var fanId = new UTF8Encoding(false, true).GetString(new ReadOnlySpan<byte>(name, nameLength));
                    if (op == 2) FanControl.SetFan(_host, new() { FanId = fanId, Mode = "manual", Pwm = value });
                    else if (op == 3) FanControl.ResetFan(_host, new() { FanId = fanId, Mode = "bios" });
                    else if (op == 4) {
                        if (FanControl.ResetFans(_host).failed > 0) return Error("RESET_FAILED: fan restoration incomplete", output, capacity, written);
                    } else return Error("INVALID_PARAM: unknown operation", output, capacity, written);
                    return 0;
                } finally { Monitor.Exit(_host); }
            }
            catch (FanControlException ex) { return Error(ex.Code + ": " + ex.Message, output, capacity, written); }
            catch (Exception ex) { return Error("INTERNAL: " + ex.Message, output, capacity, written); }
        }
    }
    private static int Error(string message, byte* output, int capacity, int* written)
    {
        var bytes = Encoding.UTF8.GetBytes(message);
        *written = Math.Min(bytes.Length, capacity);
        bytes.AsSpan(0, *written).CopyTo(new Span<byte>(output, capacity));
        return -1;
    }
}
