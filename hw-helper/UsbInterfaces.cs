using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
namespace SysPulse.HwHelper;

public static class UsbInterfaces
{
    [StructLayout(LayoutKind.Sequential)]
    private struct InterfaceData { public int Size; public Guid Class; public uint Flags; public UIntPtr Reserved; }
    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr SetupDiGetClassDevsW(ref Guid guid, string? enumerator, IntPtr parent, uint flags);
    [DllImport("setupapi.dll", SetLastError = true)]
    private static extern bool SetupDiEnumDeviceInterfaces(IntPtr set, IntPtr device, ref Guid guid, uint index, ref InterfaceData data);
    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool SetupDiGetDeviceInterfaceDetailW(IntPtr set, ref InterfaceData data, IntPtr detail, uint size, out uint needed, IntPtr device);
    [DllImport("setupapi.dll")] private static extern bool SetupDiDestroyDeviceInfoList(IntPtr set);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern SafeFileHandle CreateFileW(string path, uint access, uint share, IntPtr security, uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool DeviceIoControl(SafeFileHandle handle, uint code, byte[] input, int inputSize, byte[] output, int outputSize, out int written, IntPtr overlapped);
    private static bool Query(SafeFileHandle hub, int function, byte[] data, int minimum)
        => Query(hub, function, data, minimum, out _);
    private static bool Query(SafeFileHandle hub, int function, byte[] data, int minimum, out int size)
        => DeviceIoControl(hub, (0x22u << 16) | ((uint)function << 2), data, data.Length, data, data.Length, out size, IntPtr.Zero)
            && size >= minimum && size <= data.Length;

    public static List<BoardInterface> Read()
    {
        var ports = new List<BoardInterface>();
        var topology = new List<UsbTopology.Port>();
        var guid = new Guid("f18a0e88-c30c-11d0-8815-00a0c906bed8"); // GUID_DEVINTERFACE_USB_HUB
        var set = SetupDiGetClassDevsW(ref guid, null, IntPtr.Zero, 0x12);
        if (set == new IntPtr(-1)) throw new System.ComponentModel.Win32Exception();
        try {
            for (uint i = 0; i < 256; i++) {
                var data = new InterfaceData { Size = Marshal.SizeOf<InterfaceData>() };
                if (!SetupDiEnumDeviceInterfaces(set, IntPtr.Zero, ref guid, i, ref data)) {
                    if (Marshal.GetLastWin32Error() != 259) throw new System.ComponentModel.Win32Exception();
                    break;
                }
                SetupDiGetDeviceInterfaceDetailW(set, ref data, IntPtr.Zero, 0, out var size, IntPtr.Zero);
                if (size < 8 || size > 65536) continue;
                var detail = Marshal.AllocHGlobal((int)size);
                try {
                    Marshal.WriteInt32(detail, IntPtr.Size == 8 ? 8 : 6);
                    if (!SetupDiGetDeviceInterfaceDetailW(set, ref data, detail, size, out _, IntPtr.Zero)) continue;
                    var path = Marshal.PtrToStringUni(detail + 4)!;
                    using var hub = CreateFileW(path, 0, 3, IntPtr.Zero, 3, 0, IntPtr.Zero);
                    var info = new byte[76]; // USB_NODE_INFORMATION, packed hub descriptor
                    if (hub.IsInvalid || !Query(hub, 258, info, 7)) {
                        ports.Add(new() { Id = path, Name = $"USB 集线器 {i + 1}", Kind = "USB", Source = "usb", Status = "unknown" });
                        continue;
                    }
                    int count = info[6];
                    for (int n = 1; n <= count; n++) {
                        var connection = new byte[4096];
                        BitConverter.GetBytes(n).CopyTo(connection, 0);
                        var ok = Query(hub, 274, connection, 35);
                        // usbioctl.h packs this structure to 1 byte: status at 31, speed at 23.
                        var v2 = new byte[16];
                        BitConverter.GetBytes(n).CopyTo(v2, 0);
                        BitConverter.GetBytes(16).CopyTo(v2, 4);
                        BitConverter.GetBytes(7u).CopyTo(v2, 8);
                        var extended = Query(hub, 279, v2, 16);
                        uint protocols = extended ? BitConverter.ToUInt32(v2, 8) : 0;
                        uint flags = extended ? BitConverter.ToUInt32(v2, 12) : 0;
                        var links = new List<string>();
                        uint? properties = null;
                        bool complete = false;
                        for (ushort companion = 0; companion < 16; companion++) {
                            var connector = new byte[4096];
                            BitConverter.GetBytes(n).CopyTo(connector, 0);
                            BitConverter.GetBytes(companion).CopyTo(connector, 12);
                            if (!Query(hub, 278, connector, 18, out var connectorSize)) break;
                            var parsed = UsbTopology.ParseConnector(connector.AsSpan(0, connectorSize), n, companion);
                            if (parsed == null) break;
                            properties ??= parsed.Flags;
                            if (parsed.CompanionId != null) links.Add(parsed.CompanionId);
                            if ((parsed.Flags & 4) == 0 || parsed.CompanionId == null) { complete = true; break; }
                        }
                        var value = DescribePort(path + ":" + n, $"USB 集线器 {i + 1} / 端口 {n}",
                            ok ? connection : null, protocols, flags, (properties.GetValueOrDefault() & 8) != 0);
                        UsbCapabilities.DeviceCapability? capability = null;
                        if (value.Status == "connected") {
                            if ((flags & 4) != 0) {
                                var ssp = new byte[24];
                                BitConverter.GetBytes(n).CopyTo(ssp, 0); BitConverter.GetBytes(24).CopyTo(ssp, 4);
                                if (Query(hub, 289, ssp, 24)) value.CurrentSpeed = UsbCapabilities.OperatingSpeed(ssp, n) ?? value.CurrentSpeed;
                            }
                            capability = ReadCapability(hub, n, connection);
                            if (connection[24] != 0) value.Device = "USB 集线器 · " + value.Device;
                        }
                        topology.Add(new(value, path, (int)i + 1, n, protocols,
                            properties.HasValue ? (properties.Value & 1) != 0 : null, complete, links, capability));
                    }
                } finally { Marshal.FreeHGlobal(detail); }
            }
        } finally { SetupDiDestroyDeviceInfoList(set); }
        ports.AddRange(UsbTopology.Merge(topology));
        return ports;
    }
    private static byte[]? Descriptor(SafeFileHandle hub, int port, byte type, int length)
    {
        if (length < 2 || length > 4096) return null;
        var data = new byte[12 + length];
        BitConverter.GetBytes(port).CopyTo(data, 0);
        data[4] = 0x80; data[5] = 6; data[7] = type; // GET_DESCRIPTOR, device recipient
        BitConverter.GetBytes((ushort)length).CopyTo(data, 10);
        return Query(hub, 260, data, data.Length) ? data[12..] : null;
    }
    private static UsbCapabilities.DeviceCapability? ReadCapability(SafeFileHandle hub, int port, byte[] connection)
    {
        ushort bcdUsb = BitConverter.ToUInt16(connection, 6);
        if (bcdUsb >= 0x201) {
            var header = Descriptor(hub, port, 15, 5);
            if (header is { Length: 5 } && header[0] == 5 && header[1] == 15) {
                int size = BitConverter.ToUInt16(header, 2);
                if (size is >= 5 and <= 4096 && Descriptor(hub, port, 15, size) is { } bos) {
                    var result = UsbCapabilities.ParseBos(bos);
                    if (result != null) return result;
                }
            }
        }
        // A device qualifier explicitly describes the other USB 2 speed mode.
        // bcdUSB=2.00 by itself does not prove high-speed support.
        if (bcdUsb >= 0x200 && bcdUsb < 0x300 && connection[23] is 1 or 2 &&
            Descriptor(hub, port, 6, 10) is { } qualifier && UsbCapabilities.IsDualSpeedQualifier(qualifier))
            return new("480 Mb/s", "USB Device Qualifier · 双速设备");
        return null;
    }
    public static BoardInterface DescribePort(string id, string name, byte[]? connection,
        uint protocols, uint flags, bool isTypeC)
    {
        uint status = connection is { Length: >= 35 } ? BitConverter.ToUInt32(connection, 31) : uint.MaxValue;
        return new() {
            Id = id, Name = name, Kind = ProtocolVersion(protocols),
            Connector = isTypeC ? "USB Type-C" : null,
            // EX_V2 describes a logical protocol channel. USB 2 companions of a
            // 20 Gb/s socket still report USB 2.0; USB 3 flags give no maximum.
            // Never turn those flags into the physical socket's rated speed.
            SupportedSpeed = null,
            CurrentSpeed = status == 1 ? CurrentSpeed(connection![23], flags) : null,
            Status = ConnectionStatus(status),
            Device = status == 1 ? $"VID {BitConverter.ToUInt16(connection!, 12):X4} / PID {BitConverter.ToUInt16(connection!, 14):X4}" : null,
            Source = "usb",
        };
    }
    public static string ConnectionStatus(uint status) => status switch { 0 => "disconnected", 1 => "connected", uint.MaxValue => "unknown", _ => "error" };
    public static string ProtocolVersion(uint protocols) => (protocols & 4) != 0 ? "USB 3.x" :
        (protocols & 2) != 0 ? "USB 2.0" : (protocols & 1) != 0 ? "USB 1.1" : "USB";
    public static string? CurrentSpeed(byte speed, uint flags) => (flags & 4) != 0 ? "SuperSpeedPlus（10 Gb/s 或更高）" :
        (flags & 1) != 0 ? "5 Gb/s" : speed switch { 0 => "1.5 Mb/s", 1 => "12 Mb/s", 2 => "480 Mb/s", 3 => "5 Gb/s", _ => null };
}
