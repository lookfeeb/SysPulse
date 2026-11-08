using System.Runtime.InteropServices;
using System.Text;
namespace SysPulse.HwHelper;

/// Windows PCI bus properties describe device links, not the maximum rating
/// or presence of an otherwise unidentified motherboard socket.
public static class PcieInterfaces
{
    [StructLayout(LayoutKind.Sequential)]
    private struct DeviceInfo { public int Size; public Guid Class; public uint Instance; public UIntPtr Reserved; }
    [StructLayout(LayoutKind.Sequential)]
    private struct PropertyKey { public Guid Format; public uint Id; }
    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr SetupDiGetClassDevsW(IntPtr guid, string enumerator, IntPtr parent, uint flags);
    [DllImport("setupapi.dll", SetLastError = true)]
    private static extern bool SetupDiEnumDeviceInfo(IntPtr set, uint index, ref DeviceInfo data);
    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool SetupDiGetDevicePropertyW(IntPtr set, ref DeviceInfo info, ref PropertyKey key,
        out uint type, byte[] data, uint size, out uint needed, uint flags);
    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool SetupDiGetDeviceInstanceIdW(IntPtr set, ref DeviceInfo info, StringBuilder id, int size, out int needed);
    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool SetupDiGetDeviceRegistryPropertyW(IntPtr set, ref DeviceInfo info, uint property,
        out uint type, byte[] data, uint size, out uint needed);
    [DllImport("setupapi.dll")] private static extern bool SetupDiDestroyDeviceInfoList(IntPtr set);
    [DllImport("cfgmgr32.dll")] private static extern uint CM_Get_DevNode_Status(out uint status, out uint problem, uint instance, uint flags);
    private static uint? Property(IntPtr set, ref DeviceInfo info, uint id)
    {
        var key = new PropertyKey { Format = new Guid("3ab22e31-8264-4b4e-9af5-a8d2d8e33e62"), Id = id };
        var bytes = new byte[4];
        return SetupDiGetDevicePropertyW(set, ref info, ref key, out var type, bytes, 4, out var size, 0) && type == 7 && size == 4
            ? BitConverter.ToUInt32(bytes) : null; // DEVPROP_TYPE_UINT32
    }
    public static List<BoardInterface> Read()
    {
        var ports = new List<BoardInterface>();
        var set = SetupDiGetClassDevsW(IntPtr.Zero, "PCI", IntPtr.Zero, 6);
        if (set == new IntPtr(-1)) throw new System.ComponentModel.Win32Exception();
        try {
            for (uint i = 0; i < 4096; i++) {
                var info = new DeviceInfo { Size = Marshal.SizeOf<DeviceInfo>() };
                if (!SetupDiEnumDeviceInfo(set, i, ref info)) {
                    if(Marshal.GetLastWin32Error() != 259) throw new System.ComponentModel.Win32Exception();
                    break;
                }
                var current = Property(set, ref info, 9); var width = Property(set, ref info, 10);
                var maximum = Property(set, ref info, 11); var maxWidth = Property(set, ref info, 12);
                if (current == null && maximum == null) continue;
                var id = new StringBuilder(512);
                if (!SetupDiGetDeviceInstanceIdW(set, ref info, id, id.Capacity, out _)) continue;
                var description = new byte[2048];
                string name = "PCIe 设备链路";
                foreach (uint key in new uint[] { 12, 0 }) { // friendly name, device description
                    if (SetupDiGetDeviceRegistryPropertyW(set, ref info, key, out var type, description, (uint)description.Length, out var size)
                        && type == 1 && size > 2 && size <= description.Length) {
                        name = Encoding.Unicode.GetString(description, 0, (int)size).TrimEnd('\0'); break;
                    }
                }
                bool running = CM_Get_DevNode_Status(out var status, out var problem, info.Instance, 0) == 0 &&
                    (status & 8) != 0 && problem == 0; // DN_STARTED
                ports.Add(new() { Id = "pcie:" + id, Name = name, Kind = "PCIe 设备链路",
                    SupportedSpeed = LinkRate(maximum, maxWidth), CurrentSpeed = running ? LinkRate(current, width) : null,
                    Status = running && current is > 0 && width is > 0 ? "connected" : "unknown",
                    Device = name, Source = "pcie",
                });
            }
        } finally { SetupDiDestroyDeviceInfoList(set); }
        return ports;
    }
    public static string? LinkRate(uint? speed, uint? width)
    {
        var rate = speed switch { 1 => "2.5", 2 => "5", 3 => "8", 4 => "16", 5 => "32", 6 => "64", _ => null };
        if (rate == null) return null;
        return rate + " GT/s" + (width is >= 1 and <= 32 ? $" ×{width}" : " / lane");
    }
}
