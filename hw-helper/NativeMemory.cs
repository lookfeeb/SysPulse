using System.Runtime.InteropServices;

namespace SysPulse.HwHelper;

/// Physical memory from one Windows snapshot, without LHM's float-GB conversion.
public static class NativeMemory
{
    [StructLayout(LayoutKind.Sequential)]
    private struct MemoryStatus
    {
        public uint Length, MemoryLoad;
        public ulong TotalPhys, AvailPhys, TotalPageFile, AvailPageFile;
        public ulong TotalVirtual, AvailVirtual, AvailExtendedVirtual;
    }

    [DllImport("kernel32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GlobalMemoryStatusEx(ref MemoryStatus status);

    public static MemoryHw? Read(WmiCache metadata)
    {
        var status = new MemoryStatus { Length = (uint)Marshal.SizeOf<MemoryStatus>() };
        return GlobalMemoryStatusEx(ref status) ? FromCounters(status.TotalPhys, status.AvailPhys, metadata) : null;
    }

    public static MemoryHw? FromCounters(ulong total, ulong available, WmiCache metadata)
    {
        if (total == 0 || available > total) return null;
        var used = total - available;
        return new MemoryHw {
            TotalBytes = total, UsedBytes = used, UsedPercent = 100d * used / total,
            Modules = metadata.MemoryModules, FrequencyMhz = metadata.MemoryFrequencyMhz,
            Channels = metadata.MemoryChannels,
        };
    }
}
