using System.Buffers.Binary;
using System.Globalization;

namespace SysPulse.HwHelper;

/// Parses reported USB capabilities, never inferring speed from bcdUSB or connector shape.
public static class UsbCapabilities
{
    public sealed record DeviceCapability(string Speed, string Source);
    public static string Rate(ulong bits) => bits >= 1_000_000_000
        ? (bits / 1_000_000_000.0).ToString("0.###", CultureInfo.InvariantCulture) + " Gb/s"
        : (bits / 1_000_000.0).ToString("0.###", CultureInfo.InvariantCulture) + " Mb/s";

    private static ulong? LaneRate(uint attr)
    {
        if ((attr & 0x3f00) != 0 || ((attr >> 14) & 3) > 1) return null;
        ulong mantissa = attr >> 16;
        if (mantissa == 0) return null;
        ulong multiplier = ((attr >> 4) & 3) switch { 0 => 1, 1 => 1000, 2 => 1_000_000, _ => 1_000_000_000 };
        var bits = mantissa * multiplier;
        return bits <= 160_000_000_000 ? bits : null;
    }

    public static bool IsDualSpeedQualifier(ReadOnlySpan<byte> data)
    {
        if (data.Length != 10 || data[0] != 10 || data[1] != 6) return false;
        ushort version = BinaryPrimitives.ReadUInt16LittleEndian(data[2..]);
        return version is >= 0x200 and < 0x300 && data[7] is 8 or 16 or 32 or 64
            && data[8] > 0 && data[9] == 0;
    }

    public static string? OperatingSpeed(ReadOnlySpan<byte> data, int port)
    {
        // USB_NODE_CONNECTION_SUPERSPEEDPLUS_INFORMATION: lane counts are zero based.
        if (data.Length < 24 || BinaryPrimitives.ReadUInt32LittleEndian(data) != port ||
            BinaryPrimitives.ReadUInt32LittleEndian(data[4..]) != 24) return null;
        var rx = LaneRate(BinaryPrimitives.ReadUInt32LittleEndian(data[8..]));
        var tx = LaneRate(BinaryPrimitives.ReadUInt32LittleEndian(data[16..]));
        uint rxCount = BinaryPrimitives.ReadUInt32LittleEndian(data[12..]);
        uint txCount = BinaryPrimitives.ReadUInt32LittleEndian(data[20..]);
        if (rx == null || tx == null || rxCount > 1 || txCount > 1) return null;
        ulong receive = rx.Value * (rxCount + 1), transmit = tx.Value * (txCount + 1);
        return receive == transmit ? Rate(receive) : $"接收 {Rate(receive)} / 发送 {Rate(transmit)}";
    }

    public static DeviceCapability? ParseBos(ReadOnlySpan<byte> data)
    {
        if (data.Length < 5 || data[0] != 5 || data[1] != 15) return null;
        int total = BinaryPrimitives.ReadUInt16LittleEndian(data[2..]);
        if (total != data.Length || total > 4096) return null;
        int count = data[4], offset = 5;
        ushort? speeds = null;
        string? ssp = null;
        bool sawSsp = false;
        for (int i = 0; i < count; i++) {
            if (offset + 3 > total) return null;
            int length = data[offset];
            if (length < 3 || offset + length > total || data[offset + 1] != 16) return null;
            var item = data.Slice(offset, length);
            if (item[2] == 3) {
                if (length != 10 || speeds.HasValue) return null;
                speeds = BinaryPrimitives.ReadUInt16LittleEndian(item[4..]);
                if ((speeds.Value & 0xfff0) != 0) return null;
            } else if (item[2] == 10) {
                if (sawSsp || length < 16) return null;
                sawSsp = true;
                uint attrs = BinaryPrimitives.ReadUInt32LittleEndian(item[4..]);
                int entries = (int)(attrs & 31) + 1;
                if (12 + entries * 4 != length || (attrs >> 9) != 0) return null;
                ulong rx = 0, tx = 0;
                for (int j = 0; j < entries; j++) {
                    uint attr = BinaryPrimitives.ReadUInt32LittleEndian(item[(12 + j * 4)..]);
                    var rate = LaneRate(attr);
                    if (rate == null) return null;
                    if ((attr & 128) == 0) rx = Math.Max(rx, rate.Value); else tx = Math.Max(tx, rate.Value);
                }
                if (rx == 0 || tx == 0) return null;
                // wFunctionalitySupport contains MINIMUM lane counts, not maxima.
                // Do not multiply these into a fictitious 10/20 Gb/s device maximum.
                ssp = rx == tx ? $"每通道 {Rate(rx)}（最大通道数未报告）"
                    : $"每通道接收 {Rate(rx)} / 发送 {Rate(tx)}（最大通道数未报告）";
            }
            offset += length;
        }
        if (offset != total) return null;
        if (sawSsp) return ssp == null ? null : new(ssp, "USB BOS · SuperSpeedPlus");
        if (!speeds.HasValue) return null;
        ulong max = (speeds.Value & 8) != 0 ? 5_000_000_000UL : (speeds.Value & 4) != 0 ? 480_000_000UL :
            (speeds.Value & 2) != 0 ? 12_000_000UL : (speeds.Value & 1) != 0 ? 1_500_000UL : 0UL;
        return max == 0 ? null : new(Rate(max), "USB BOS · 支持速率位图");
    }
}
