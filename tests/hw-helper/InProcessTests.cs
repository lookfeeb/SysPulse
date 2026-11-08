using System.Text;
using SysPulse.HwHelper;
public static class InProcessTests
{
    private static void Check(bool value) { if (!value) throw new Exception("In-process regression failed"); }
    public static void Run(Action<string, Action> test)
    {
        test("PCIe link encoding keeps per-lane rates and width distinct from throughput", () => {
            Check(PcieInterfaces.LinkRate(4, 16) == "16 GT/s ×16");
            Check(PcieInterfaces.LinkRate(3, null) == "8 GT/s / lane");
            Check(PcieInterfaces.LinkRate(null, 16) == null);
            Check(PcieInterfaces.LinkRate(99, 16) == null);
        });
        test("firmware slot generations are explicit; M.2 does not imply NVMe or a speed", () => {
            Check(FirmwareInterfaces.SlotType(0x17) == ("M.2", null));
            Check(FirmwareInterfaces.SlotType(0xa5) == ("PCIe", null));
            Check(FirmwareInterfaces.SlotType(0xbd) == ("PCIe 4.0", "16 GT/s / lane"));
            Check(FirmwareInterfaces.SlotType(0xc3) == ("PCIe 5.0", "32 GT/s / lane"));
        });
        test("SMBIOS connectors and boot slot occupation do not become live connections", () => {
            byte[] data = [8,9,1,0,1,0x23,2,0x23,0x10, .. Encoding.UTF8.GetBytes("内部\0USB-C\0\0"),
                9,13,2,0,1,0xbd,13,4,0,0,0,0,0, .. Encoding.UTF8.GetBytes("PCIEX16\0\0"), 127,4,0,0,0,0];
            var (ports, _, _) = FirmwareInterfaces.Parse(data);
            Check(ports.Count == 2 && ports[0].Connector == "USB Type-C" && ports[0].Name == "USB-C");
            Check(ports[0].Status == "unknown" && ports[0].SupportedSpeed == null);
            Check(ports[1].Status == "in-use" && ports[1].Source == "firmware-slot" && ports[1].Connector == "x16");
        });
        test("truncated SMBIOS structures are rejected", () => {
            foreach (var bytes in new byte[][] { [9,13,0,0], [8,9,0,0,0,0,0,0,0,65] }) {
                bool failed = false; try { FirmwareInterfaces.Parse(bytes); } catch (InvalidDataException) { failed = true; }
                Check(failed);
            }
        });
        test("USB port capabilities, negotiated speed and error state stay distinct", () => {
            Check(UsbInterfaces.ProtocolVersion(2) == "USB 2.0");
            Check(UsbInterfaces.ProtocolVersion(7) == "USB 3.x");
            Check(UsbInterfaces.ProtocolVersion(0) == "USB");
            Check(UsbInterfaces.CurrentSpeed(1, 2) == "12 Mb/s"); // capable device is not necessarily operating at SS
            Check(UsbInterfaces.CurrentSpeed(2, 4)!.Contains("10 Gb/s"));
            Check(UsbInterfaces.ConnectionStatus(0) == "disconnected");
            Check(UsbInterfaces.ConnectionStatus(2) == "error");
            Check(UsbInterfaces.ConnectionStatus(uint.MaxValue) == "unknown");
        });
        test("USB companion protocols never become physical connector maximum speeds", () => {
            var raw = new byte[35]; raw[23] = 2;
            BitConverter.GetBytes(1u).CopyTo(raw, 31);
            BitConverter.GetBytes((ushort)0x1234).CopyTo(raw, 12);
            BitConverter.GetBytes((ushort)0xabcd).CopyTo(raw, 14);
            var port = UsbInterfaces.DescribePort("hub:1", "USB channel", raw, 2, 0, true);
            Check(port.Kind == "USB 2.0" && port.Connector == "USB Type-C");
            Check(port.SupportedSpeed == null && port.CurrentSpeed == "480 Mb/s");
            Check(port.Device == "VID 1234 / PID ABCD" && port.Status == "connected");
            var ss = UsbInterfaces.DescribePort("hub:2", "USB channel", null, 4, 15, true);
            Check(ss.Kind == "USB 3.x" && ss.SupportedSpeed == null && ss.CurrentSpeed == null && ss.Status == "unknown");
        });
        test("binary ABI preserves Unicode, null, zero and 64-bit counters across buffer reuse", () => {
            var snapshot = Fixture();
            using var encoder = new SnapshotBinary();
            var first = encoder.Encode(snapshot).ToArray();
            Check(Encoding.ASCII.GetString(first, 0, 4) == "SPW2");
            using var reader = new BinaryReader(new MemoryStream(first));
            reader.ReadUInt32(); Check(reader.ReadInt64() == 123); reader.ReadInt64(); reader.ReadInt64();
            Check(reader.ReadBoolean()); int length = reader.ReadInt32();
            Check(Encoding.UTF8.GetString(reader.ReadBytes(length)) == "处理器");
            Check(reader.ReadBoolean() && reader.ReadDouble() == 0);
            encoder.Encode(new HwSnapshot { Cpu = new() { Name = new string('x', 20000) } });
            Check(encoder.Encode(snapshot).SequenceEqual(first));
            if (Environment.GetEnvironmentVariable("HW_ABI_FIXTURE") is string file) File.WriteAllBytes(file, first);
        });
    }
    public static HwSnapshot Fixture() => new() {
        TimestampMs = 123, SampleAgeMs = 8, StorageAgeMs = 42,
        Cpu = new() { Name = "处理器", PackageTempC = 0, PerCoreTempsC = [null, 0, 45], PerCoreUsage = [1, null], TotalUsage = 50, FrequencyMhz = 4200 },
        Gpus = [new() { Name = "GPU", TempC = 60, Index = 2, MemUsedMb = 0 }],
        Memory = new() { TotalBytes = 1099511627776, UsedBytes = 4096, SwapTotalBytes = ulong.MaxValue, Modules = [new() { Slot = "DIMM 1", CapacityBytes = 1099511627776 }] },
        Disks = [new() { Identifier = "disk:零", Model = "盘", UsedBytes = null, TotalBytes = 2000000000000, ReadBytesPerSec = 0 }],
        Motherboard = new() { Model = "测试板", InterfacesAgeMs = 500,
            TemperaturesC = [new() { Name = "主板", Value = 45 }],
            Interfaces = [new() { Id = "usb:1", Name = "USB 接口", Kind = "USB", Source = "usb", Status = "disconnected", SupportedSpeed = "480 Mb/s",
                Usb = new() { LogicalPortCount = 2, UserConnectable = true, MaximumSpeedSource = "测试来源", DeviceCapability = "5 Gb/s", DeviceCapabilitySource = "USB BOS", TopologyNote = "伴随端口" } }] },
        Fans = [new() { Id = "fan:1", Name = "风扇", Rpm = 0, PwmPercent = null, Controllable = false }],
    };
}
