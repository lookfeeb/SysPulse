using System.Text;
namespace SysPulse.HwHelper;

/// SPW2 snapshot: little endian, u32 byte-length UTF-8, one-byte presence flags.
/// Only the ABI worker owns this reusable buffer. No JSON or managed references cross FFI.
public sealed class SnapshotBinary : IDisposable
{
    private readonly MemoryStream _buffer = new(16384);
    private readonly BinaryWriter _writer;
    public SnapshotBinary() => _writer = new(_buffer, new UTF8Encoding(false, true), true);
    public ReadOnlySpan<byte> Encode(HwSnapshot s)
    {
        _buffer.Position = 0; _buffer.SetLength(0);
        var w = _writer;
        w.Write(0x32575053u); // SPW2: adds optional USB capability provenance.
        w.Write(s.TimestampMs); w.Write(s.SampleAgeMs); w.Write(s.StorageAgeMs);
        Opt(s.Cpu, c => {
            Text(c.Name); Num(c.PackageTempC); List(c.PerCoreTempsC, Num); List(c.PerCoreUsage, Num);
            Num(c.TotalUsage); Num(c.FrequencyMhz); Num(c.PowerW);
        });
        List(s.Gpus, g => {
            w.Write(g.Index); Text(g.Name); Text(g.Vendor); Num(g.UsagePercent); Num(g.MemUsedMb);
            Num(g.MemTotalMb); Num(g.TempC); Num(g.PowerW); Num(g.FanRpm); Num(g.FanPwm);
        });
        Opt(s.Memory, m => {
            w.Write(m.TotalBytes); w.Write(m.UsedBytes); w.Write(m.UsedPercent);
            w.Write(m.SwapTotalBytes); w.Write(m.SwapUsedBytes);
            List(m.Modules, x => { Text(x.Slot); w.Write(x.CapacityBytes); Num(x.SpeedMtps); Text(x.Manufacturer); Text(x.PartNumber); });
            Num(m.FrequencyMhz); w.Write(m.Channels.HasValue); if (m.Channels.HasValue) w.Write(m.Channels.Value);
        });
        List(s.Disks, d => {
            w.Write(d.Index); Text(d.Model); Text(d.Bus); Num(d.TempC); Text(d.Health);
            Num(d.ReadBytesPerSec); Num(d.WriteBytesPerSec); w.Write(d.TotalBytes);
            w.Write(d.UsedBytes.HasValue); if(d.UsedBytes.HasValue) w.Write(d.UsedBytes.Value); Text(d.Identifier);
        });
        Opt(s.Motherboard, m => {
            Text(m.Vendor); Text(m.Model); MaybeText(m.SuperIo);
            List(m.TemperaturesC, Named); List(m.VoltagesV, Named);
            w.Write(m.InterfacesAgeMs);
            List(m.Interfaces, p => { Text(p.Id); Text(p.Name); Text(p.Kind); MaybeText(p.Connector);
                MaybeText(p.SupportedSpeed); MaybeText(p.CurrentSpeed); Text(p.Status); MaybeText(p.Device); Text(p.Source);
                Opt(p.Usb, u => {
                    w.Write(u.LogicalPortCount); w.Write(u.UserConnectable.HasValue);
                    if (u.UserConnectable.HasValue) w.Write(u.UserConnectable.Value);
                    MaybeText(u.MaximumSpeedSource); MaybeText(u.DeviceCapability);
                    MaybeText(u.DeviceCapabilitySource); MaybeText(u.TopologyNote);
                });
            });
        });
        List(s.Fans, f => { Text(f.Id); Text(f.Name); Num(f.Rpm); Num(f.PwmPercent); w.Write(f.Controllable); });
        w.Flush();
        if (_buffer.Length > 4 * 1024 * 1024) throw new InvalidDataException("Snapshot exceeds ABI limit");
        return _buffer.GetBuffer().AsSpan(0, (int)_buffer.Length);
    }
    private void Text(string s) { _writer.Write(Encoding.UTF8.GetByteCount(s)); _writer.Write(s.AsSpan()); }
    private void MaybeText(string? s) { _writer.Write(s != null); if(s != null) Text(s); }
    private void Num(double? x) { _writer.Write(x.HasValue); if(x.HasValue) _writer.Write(x.Value); }
    private void Opt<T>(T? x, Action<T> write) where T : class { _writer.Write(x != null); if(x != null) write(x); }
    private void List<T>(List<T> list, Action<T> write) { _writer.Write(list.Count); foreach(var x in list) write(x); }
    private void Named(NamedValue n) { Text(n.Name); _writer.Write(n.Value); Text(n.Identifier); }
    public void Dispose() { _writer.Dispose(); _buffer.Dispose(); }
}
