using SysPulse.HwHelper;
using System.Text;

public static class UsbCapabilityTests
{
    private static void Check(bool ok) { if (!ok) throw new Exception("USB capability regression failed"); }
    public static void Run(Action<string, Action> test)
    {
        test("USB 2 dual-speed evidence requires a valid qualifier", () => {
            byte[] qualifier = [10,6,0,2,0,0,0,64,1,0];
            Check(UsbCapabilities.IsDualSpeedQualifier(qualifier));
            Check(!UsbCapabilities.IsDualSpeedQualifier(qualifier.AsSpan(0,9)));
            qualifier[9]=1;Check(!UsbCapabilities.IsDualSpeedQualifier(qualifier));
            qualifier[9]=0;qualifier[8]=0;Check(!UsbCapabilities.IsDualSpeedQualifier(qualifier));
            qualifier[8]=1;qualifier[7]=0;Check(!UsbCapabilities.IsDualSpeedQualifier(qualifier));
        });
        test("BOS capabilities describe the device, not the connected port or bcdUSB version", () => {
            byte[] bos = [5,15,15,0,1,10,16,3,0,14,0,1,8,190,0];
            Check(UsbCapabilities.ParseBos(bos)?.Speed == "5 Gb/s");
            Check(UsbCapabilities.ParseBos([5,15,12,0,1,7,16,2,6,0,0,0]) == null);
            for (int i = 0; i < bos.Length; i++) Check(UsbCapabilities.ParseBos(bos.AsSpan(0,i)) == null);
            bos[5] = 0; Check(UsbCapabilities.ParseBos(bos) == null);
        });
        test("SuperSpeedPlus descriptor minimum lanes do not become a device maximum", () => {
            var bos = new byte[25]; bos[0]=5;bos[1]=15;bos[2]=25;bos[4]=1;
            bos[5]=20;bos[6]=16;bos[7]=10;bos[9]=1;
            bos[14]=0x22; // min RX/TX lanes: cannot be used as MAX lanes
            BitConverter.GetBytes((10u<<16)|(1u<<14)|0x30u).CopyTo(bos,17);
            BitConverter.GetBytes((10u<<16)|(1u<<14)|0xb0u).CopyTo(bos,21);
            Check(UsbCapabilities.ParseBos(bos)?.Speed == "每通道 10 Gb/s（最大通道数未报告）");
            var current = new byte[24]; BitConverter.GetBytes(3).CopyTo(current,0);BitConverter.GetBytes(24).CopyTo(current,4);
            BitConverter.GetBytes((10u<<16)|(1u<<14)|0x30u).CopyTo(current,8);
            BitConverter.GetBytes((10u<<16)|(1u<<14)|0xb0u).CopyTo(current,16);
            Check(UsbCapabilities.OperatingSpeed(current,3) == "10 Gb/s");
            current[12]=1;current[20]=1;
            Check(UsbCapabilities.OperatingSpeed(current,3) == "20 Gb/s");
            Check(UsbCapabilities.OperatingSpeed(current,4) == null);
            current[12]=255;Check(UsbCapabilities.OperatingSpeed(current,3) == null);
        });
        test("connector responses require a complete terminated name and matching query identity", () => {
            byte[] name=Encoding.Unicode.GetBytes("USB#HUB\0"); var data=new byte[16+name.Length];
            BitConverter.GetBytes(1).CopyTo(data,0);BitConverter.GetBytes(data.Length).CopyTo(data,4);
            data[8]=9;data[14]=2;name.CopyTo(data,16);
            Check(UsbTopology.ParseConnector(data,1,0)?.CompanionId == "USB#HUB:2");
            Check(UsbTopology.ParseConnector(data,2,0) == null);
            Check(UsbTopology.ParseConnector(data,1,1) == null);
            Check(UsbTopology.ParseConnector(data.AsSpan(0,data.Length-1),1,0) == null);
        });
        test("reciprocal USB2/3 companions merge without inheriting USB2 or device maximum", () => {
            var a=Port(1,2,"disconnected",["USB#HUB:2"]);var b=Port(2,4,"connected",["USB#HUB:1"]);
            b.Value.Connector="USB Type-C";b.Value.CurrentSpeed="5 Gb/s";
            var merged=UsbTopology.Merge([a,b]);
            Check(merged.Count==1 && merged[0].Usb?.LogicalPortCount==2);
            Check(merged[0].Kind=="USB 3.x" && merged[0].Connector=="USB Type-C");
            Check(merged[0].Status=="connected" && merged[0].CurrentSpeed=="5 Gb/s");
            Check(merged[0].SupportedSpeed==null && merged[0].Usb?.DeviceCapability=="5 Gb/s");
            b.Value.Status="disconnected";
            merged=UsbTopology.Merge([b,a]);
            Check(merged[0].Status=="disconnected" && merged[0].CurrentSpeed==null && merged[0].Usb?.DeviceCapability==null);
            Check(merged[0].Id=="USB#HUB:1");
        });
        test("missing or one-way companions cannot establish maximum path speed", () => {
            var a=Port(1,2,"disconnected",["USB#HUB:2"]);var b=Port(2,4,"disconnected",[]);
            Check(UsbTopology.Merge([a,b]).Count==2);
            Check(UsbTopology.Merge([a])[0].SupportedSpeed==null);
            var single=Port(1,2,"disconnected",[]);
            Check(UsbTopology.Merge([single])[0].SupportedSpeed=="480 Mb/s");
            Check(UsbTopology.Merge([single with { MappingComplete=false }])[0].SupportedSpeed==null);
            Check(UsbTopology.Merge([single with { Protocols=0 }])[0].SupportedSpeed==null);
            b = b with { Protocols=0, Companions=["USB#HUB:1"] };
            Check(UsbTopology.Merge([a,b])[0].SupportedSpeed==null);
        });
    }
    private static UsbTopology.Port Port(int n,uint protocols,string status,List<string> links) => new(
        new(){Id="USB#HUB:"+n,Name="USB "+n,Source="usb",Kind="USB",Status=status},
        @"\\?\USB#HUB",1,n,protocols,true,true,links,new("5 Gb/s","USB BOS"));
}
