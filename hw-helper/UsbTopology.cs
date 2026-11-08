using System.Buffers.Binary;
using System.Text;

namespace SysPulse.HwHelper;

public static class UsbTopology
{
    public sealed record ConnectorInfo(uint Flags, string? CompanionId);
    public sealed record Port(BoardInterface Value, string Hub, int HubNumber, int Number,
        uint Protocols, bool? UserConnectable, bool MappingComplete, List<string> Companions,
        UsbCapabilities.DeviceCapability? DeviceCapability);

    public static string NormalizeHub(string path)
    {
        var name = path.TrimEnd('\0');
        if (name.StartsWith(@"\\?\") || name.StartsWith(@"\\.\")) name = name[4..];
        return name.ToUpperInvariant();
    }
    public static string PortId(string hub, int port) => NormalizeHub(hub) + ":" + port;
    public static ConnectorInfo? ParseConnector(ReadOnlySpan<byte> data, int port, int companionIndex)
    {
        if (data.Length < 18 || BinaryPrimitives.ReadUInt32LittleEndian(data) != port ||
            BinaryPrimitives.ReadUInt16LittleEndian(data[12..]) != companionIndex) return null;
        uint length = BinaryPrimitives.ReadUInt32LittleEndian(data[4..]);
        if (length < 18 || length > data.Length || (length & 1) != 0) return null;
        uint flags = BinaryPrimitives.ReadUInt32LittleEndian(data[8..]);
        if ((flags & ~15u) != 0) return null;
        int number = BinaryPrimitives.ReadUInt16LittleEndian(data[14..]);
        var tail = data.Slice(16, (int)length - 16);
        int end = -1;
        for (int i = 0; i + 1 < tail.Length; i += 2) if (tail[i] == 0 && tail[i + 1] == 0) { end = i; break; }
        if (end < 0) return null;
        string name = Encoding.Unicode.GetString(tail[..end]);
        if ((number == 0) != (name.Length == 0) || number > 255) return null;
        return new(flags, number == 0 ? null : PortId(name, number));
    }

    public static List<BoardInterface> Merge(List<Port> ports)
    {
        var index = ports.ToDictionary(p => PortId(p.Hub, p.Number), StringComparer.OrdinalIgnoreCase);
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var output = new List<BoardInterface>();
        foreach (var first in ports) {
            var key = PortId(first.Hub, first.Number);
            if (!seen.Add(key)) continue;
            var group = new List<Port> { first };
            for (int i = 0; i < group.Count; i++) {
                var item = group[i];
                foreach (var link in item.Companions) {
                    if (!index.TryGetValue(link, out var other) ||
                        !other.Companions.Contains(PortId(item.Hub, item.Number), StringComparer.OrdinalIgnoreCase)) continue;
                    if (group.Count < 256 && seen.Add(link)) group.Add(other);
                }
            }
            group.Sort((a, b) => StringComparer.OrdinalIgnoreCase.Compare(PortId(a.Hub, a.Number), PortId(b.Hub, b.Number)));
            var primary = group[0];
            var value = primary.Value;
            var active = group.Where(p => p.Value.Status == "connected").ToList();
            uint protocols = group.Aggregate(0u, (bits, p) => bits | p.Protocols);
            var members = group.Select(p => PortId(p.Hub, p.Number)).ToHashSet(StringComparer.OrdinalIgnoreCase);
            bool complete = group.All(p => p.MappingComplete && p.Companions.All(id =>
                members.Contains(id) && index.TryGetValue(id, out var other) &&
                other.Companions.Contains(PortId(p.Hub, p.Number), StringComparer.OrdinalIgnoreCase)));
            string? deviceSpeed = Join(active.Select(p => p.DeviceCapability?.Speed));
            string? deviceSource = Join(active.Select(p => p.DeviceCapability?.Source));
            // Only a complete USB 1/2 topology gives a bounded system USB path.
            // The USB 3 protocol bit does not reveal a 5/10/20 Gb/s maximum.
            string? portSpeed = complete && group.All(p => p.Protocols != 0) && (protocols & ~3u) == 0
                ? ((protocols & 2) != 0 ? "480 Mb/s" : "12 Mb/s") : null;
            output.Add(new() {
                Id = PortId(primary.Hub, primary.Number),
                Name = group.Count == 1 ? value.Name : "USB 端口组 · " + string.Join(" + ",
                    group.OrderBy(p => p.HubNumber).ThenBy(p => p.Number).Select(p => $"{p.HubNumber}/{p.Number}")),
                Kind = UsbInterfaces.ProtocolVersion(protocols),
                Connector = group.Any(p => p.Value.Connector == "USB Type-C") ? "USB Type-C" : value.Connector,
                SupportedSpeed = portSpeed,
                CurrentSpeed = Join(active.Select(p => p.Value.CurrentSpeed ?? "未报告")),
                Status = active.Count > 0 ? "connected" : group.Any(p => p.Value.Status == "error") ? "error" :
                    group.All(p => p.Value.Status == "disconnected") ? "disconnected" : "unknown",
                Device = Join(active.Select(p => p.Value.Device)), Source = "usb",
                Usb = new() {
                    LogicalPortCount = group.Count,
                    UserConnectable = group.All(p => p.UserConnectable.HasValue) ? group.Any(p => p.UserConnectable == true) : null,
                    MaximumSpeedSource = portSpeed == null ? null : "USB 协议 + 完整伴随端口映射",
                    DeviceCapability = deviceSpeed, DeviceCapabilitySource = deviceSource,
                    TopologyNote = group.Count > 1 ? $"已合并 {group.Count} 条伴随记录" : complete ? "无伴随端口" : "伴随映射未完整报告",
                },
            });
        }
        return output;
    }
    private static string? Join(IEnumerable<string?> values)
    {
        var items = values.Where(x => !string.IsNullOrWhiteSpace(x)).Distinct().ToArray();
        return items.Length == 0 ? null : string.Join(" / ", items);
    }
}
