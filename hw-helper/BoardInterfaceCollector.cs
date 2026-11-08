using System.Diagnostics;
using System.Net.NetworkInformation;
using System.Runtime.InteropServices;
using System.Text;

namespace SysPulse.HwHelper;

/// Independent 5s topology collector. Firmware describes boot-time inventory;
/// only live USB/network observations claim a present connection.
public sealed class BoardInterfaceCollector : IDisposable
{
    private sealed record Publication(List<BoardInterface> Ports, string Vendor, string Model, long Started);
    private Publication? _latest;
    private readonly ManualResetEventSlim _stop = new(false);
    private readonly Thread _thread;
    public BoardInterfaceCollector()
    {
        _thread = new Thread(Collect) { IsBackground = true, Name = "board-interfaces" };
        _thread.Start();
    }
    private void Collect()
    {
        var firmware = new List<BoardInterface>();
        string vendor = "", model = "";
        try { (firmware, vendor, model) = FirmwareInterfaces.Read(); }
        catch (Exception ex) { Console.Error.WriteLine("[hw-helper] firmware interfaces: " + ex.Message); }
        while (!_stop.IsSet) {
            var started = Stopwatch.GetTimestamp();
            var ports = new List<BoardInterface>(firmware);
            try { ports.AddRange(UsbInterfaces.Read()); }
            catch (Exception ex) { Console.Error.WriteLine("[hw-helper] USB topology: " + ex.Message); }
            try { ports.AddRange(PcieInterfaces.Read()); }
            catch (Exception ex) { Console.Error.WriteLine("[hw-helper] PCIe links: " + ex.Message); }
            try {
                foreach (var nic in NetworkInterface.GetAllNetworkInterfaces().Where(n =>
                    n.NetworkInterfaceType is NetworkInterfaceType.Ethernet or NetworkInterfaceType.Wireless80211))
                    ports.Add(new() {
                        Id = "nic:" + nic.Id, Name = nic.Name, Kind = nic.NetworkInterfaceType == NetworkInterfaceType.Ethernet ? "以太网适配器" : "无线网卡",
                        Status = nic.OperationalStatus == OperationalStatus.Up ? "connected" :
                            nic.OperationalStatus == OperationalStatus.Down ? "disconnected" : "unknown",
                        CurrentSpeed = nic.OperationalStatus == OperationalStatus.Up && nic.Speed > 0 ? Rate(nic.Speed) : null,
                        Device = nic.Description, Source = "network",
                    });
            } catch (Exception ex) { Console.Error.WriteLine("[hw-helper] network interfaces: " + ex.Message); }
            Interlocked.Exchange(ref _latest, new(ports, vendor, model, started));
            if (_stop.Wait(HardwareSampler.NextDelayMs(Stopwatch.GetElapsedTime(started), 5000))) break;
        }
    }
    public static string Rate(long bits) => bits >= 1_000_000_000 ? $"{bits / 1_000_000_000.0:0.##} Gb/s" : $"{bits / 1_000_000.0:0.##} Mb/s";
    public void Attach(HwSnapshot snapshot)
    {
        var p = Volatile.Read(ref _latest);
        if (p == null) return;
        var age = (long)Stopwatch.GetElapsedTime(p.Started).TotalMilliseconds;
        var old = snapshot.Motherboard;
        // Copy the wrapper; cached sensor publications must remain immutable.
        snapshot.Motherboard = new() {
            Model = !string.IsNullOrEmpty(old?.Model) ? old.Model : p.Model, Vendor = p.Vendor,
            SuperIo = old?.SuperIo, TemperaturesC = old?.TemperaturesC ?? new(), VoltagesV = old?.VoltagesV ?? new(),
            Interfaces = age <= 15000 ? p.Ports : new(), InterfacesAgeMs = age,
        };
    }
    public void Dispose() { _stop.Set(); _thread.Join(); _stop.Dispose(); }
}

public static class FirmwareInterfaces
{
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern uint GetSystemFirmwareTable(uint provider, uint id, byte[]? buffer, uint size);
    public static (List<BoardInterface>, string, string) Read()
    {
        const uint rsmb = 0x52534D42;
        var length = GetSystemFirmwareTable(rsmb, 0, null, 0);
        if (length < 8 || length > 4 * 1024 * 1024) return (new(), "", "");
        var buffer = new byte[length];
        if (GetSystemFirmwareTable(rsmb, 0, buffer, length) != length) throw new IOException("Firmware changed while reading");
        var payload = BitConverter.ToUInt32(buffer, 4);
        if (payload > length - 8) throw new InvalidDataException("Truncated SMBIOS table");
        return Parse(buffer.AsSpan(8, (int)payload));
    }
    // SMBIOS DSP0134 type 2/8/9. Connector and slot enums are firmware facts,
    // never inferred from a marketing name or attached device capability.
    public static (List<BoardInterface>, string, string) Parse(ReadOnlySpan<byte> data)
    {
        var ports = new List<BoardInterface>();
        string vendor = "", model = "";
        int offset = 0;
        while (offset + 4 <= data.Length) {
            int type = data[offset], length = data[offset + 1];
            if (length < 4 || offset + length > data.Length) throw new InvalidDataException("Invalid SMBIOS structure");
            int end = offset + length;
            while (end + 1 < data.Length && (data[end] != 0 || data[end + 1] != 0)) end++;
            if (end + 1 >= data.Length) throw new InvalidDataException("Unterminated SMBIOS strings");
            var strings = Encoding.UTF8.GetString(data.Slice(offset + length, end - offset - length)).Split('\0');
            string S(byte i) => i > 0 && i <= strings.Length ? strings[i - 1].Trim() : "";
            var d = data.Slice(offset, length);
            var id = $"smbios:{type}:{BitConverter.ToUInt16(d.Slice(2, 2)):x4}";
            if (type == 2 && length >= 6) { vendor = S(d[4]); model = S(d[5]); }
            if (type == 8 && length >= 9) {
                var name = S(d[6]); if(name.Length == 0) name = S(d[4]);
                ports.Add(new() { Id = id, Name = name.Length > 0 ? name : "未命名连接器",
                    Kind = PortType(d[8]), Connector = Connector(d[7] != 0 ? d[7] : d[5]), Source = "firmware-connector" });
            }
            if (type == 9 && length >= 13) {
                var (kind, speed) = SlotType(d[5]);
                var width = d[6] switch { 8 => "x1", 9 => "x2", 10 => "x4", 11 => "x8", 12 => "x12", 13 => "x16", 14 => "x32", _ => null };
                ports.Add(new() { Id = id, Name = S(d[4]), Kind = kind, Connector = width, SupportedSpeed = speed,
                    Status = d[7] switch { 3 => "available", 4 => "in-use", 5 => "unavailable", _ => "unknown" }, Source = "firmware-slot" });
            }
            if(type == 127) break;
            offset = end + 2;
        }
        return (ports, vendor, model);
    }
    public static (string Kind, string? Speed) SlotType(byte type)
    {
        int gen = type switch { >= 0xab and <= 0xb0 => 2, >= 0xb1 and <= 0xb6 => 3,
            >= 0xb8 and <= 0xbd => 4, >= 0xbe and <= 0xc3 => 5, _ => 0 };
        if(gen > 0) return ($"PCIe {gen}.0", gen switch { 2 => "5 GT/s / lane", 3 => "8 GT/s / lane", 4 => "16 GT/s / lane", _ => "32 GT/s / lane" });
        return type switch { >= 0xa5 and <= 0xaa => ("PCIe", null), >= 0x14 and <= 0x17 => ("M.2", null),
            0x1f => ("U.2 / PCIe 2.0", "5 GT/s / lane"), 0x20 => ("U.2 / PCIe 3.0", "8 GT/s / lane"),
            0x24 => ("U.2 / PCIe 4.0", "16 GT/s / lane"), 0x25 => ("U.2 / PCIe 5.0", "32 GT/s / lane"),
            0xc4 => ("PCIe 6+", null), 6 => ("PCI", null), _ => ($"系统插槽 (0x{type:x2})", null) };
    }
    private static string PortType(byte type) => type switch {
        0x10 => "USB", 0x11 => "IEEE 1394", 0x1c => "视频", 0x1d => "音频", 0x1f => "网络",
        0x20 => "SATA", 0x21 => "SAS", 0x22 => "DisplayPort 多功能接口", 0x23 => "Thunderbolt",
        0x0d => "键盘", 0x0e => "鼠标", >= 6 and <= 9 => "串口", >= 1 and <= 5 => "并口",
        _ => "主板连接器" };
    private static string? Connector(byte type) => type switch {
        0x0b => "RJ-45", 0x0a => "RJ-11", 0x0f => "PS/2", 0x12 => "USB", 0x1f => "音频插孔",
        0x21 => "IEEE 1394", 0x22 => "SAS/SATA", 0x23 => "USB Type-C", _ => null };
}
