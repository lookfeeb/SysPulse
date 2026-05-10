using LibreHardwareMonitor.Hardware;

namespace SysPulse.HwHelper;

public static class FanControl
{
    public static object SetFan(ComputerHost host, RequestParams? args)
    {
        if (args?.FanId is null || string.IsNullOrWhiteSpace(args.FanId))
            throw new FanControlException(ErrorCodes.InvalidParam, "fanId is required");

        if (args.Mode is null)
            throw new FanControlException(ErrorCodes.InvalidParam, "mode is required");

        var target = FindControl(host, args.FanId);
        if (target is null)
            throw new FanControlException(ErrorCodes.Unsupported, $"fan is not controllable: {args.FanId}");

        if (args.Mode.Equals("bios", StringComparison.OrdinalIgnoreCase)
            || args.Mode.Equals("default", StringComparison.OrdinalIgnoreCase))
        {
            TrySetDefault(target);
            return new { fanId = args.FanId, mode = "bios" };
        }

        if (!args.Mode.Equals("manual", StringComparison.OrdinalIgnoreCase))
            throw new FanControlException(ErrorCodes.InvalidParam, $"unsupported mode: {args.Mode}");

        if (args.Pwm is null || double.IsNaN(args.Pwm.Value) || args.Pwm < 0 || args.Pwm > 100)
            throw new FanControlException(ErrorCodes.InvalidParam, "pwm must be in 0..100");

        var pwm = (float)args.Pwm.Value;
        if (pwm < target.MinSoftwareValue || pwm > target.MaxSoftwareValue)
            throw new FanControlException(
                ErrorCodes.InvalidParam,
                $"pwm must be in {target.MinSoftwareValue:0}..{target.MaxSoftwareValue:0} for this fan");

        TrySetSoftware(target, pwm);
        return new { fanId = args.FanId, mode = "manual", pwm = args.Pwm.Value };
    }

    public static object ResetFan(ComputerHost host, RequestParams? args)
    {
        if (args?.FanId is null || string.IsNullOrWhiteSpace(args.FanId))
            throw new FanControlException(ErrorCodes.InvalidParam, "fanId is required");

        var target = FindControl(host, args.FanId);
        if (target is null)
            throw new FanControlException(ErrorCodes.Unsupported, $"fan is not controllable: {args.FanId}");

        TrySetDefault(target);
        return new { fanId = args.FanId, mode = "bios" };
    }

    public static object ResetFans(ComputerHost host)
    {
        var count = 0;
        foreach (var control in AllControls(host))
        {
            try
            {
                control.SetDefault();
                count++;
            }
            catch
            {
                // Keep reset_all best-effort. A single bad controller should not
                // prevent other fans from returning to firmware control.
            }
        }

        return new { resetCount = count };
    }

    private static IControl? FindControl(ComputerHost host, string fanId)
    {
        host.Refresh();

        foreach (var hw in AllHardware(host))
        {
            var fan = hw.Sensors.FirstOrDefault(s =>
                s.SensorType == SensorType.Fan && s.Identifier.ToString() == fanId);
            if (fan?.Control != null) return fan.Control;

            if (fan != null)
            {
                var controlSensor = hw.Sensors.FirstOrDefault(s =>
                    s.SensorType == SensorType.Control && s.Index == fan.Index);
                if (controlSensor?.Control != null) return controlSensor.Control;
            }

            var directControl = hw.Sensors.FirstOrDefault(s =>
                s.SensorType == SensorType.Control && s.Identifier.ToString() == fanId);
            if (directControl?.Control != null) return directControl.Control;
        }

        return null;
    }

    private static void TrySetSoftware(IControl control, float pwm)
    {
        try
        {
            control.SetSoftware(pwm);
        }
        catch (Exception ex)
        {
            throw new FanControlException(ErrorCodes.EcWriteFail, ex.Message);
        }
    }

    private static void TrySetDefault(IControl control)
    {
        try
        {
            control.SetDefault();
        }
        catch (Exception ex)
        {
            throw new FanControlException(ErrorCodes.EcWriteFail, ex.Message);
        }
    }

    private static IEnumerable<IControl> AllControls(ComputerHost host)
    {
        host.Refresh();
        foreach (var hw in AllHardware(host))
        {
            foreach (var sensor in hw.Sensors)
            {
                if (sensor.Control != null) yield return sensor.Control;
            }
        }
    }

    private static IEnumerable<IHardware> AllHardware(ComputerHost host)
    {
        foreach (var hw in host.Inner.Hardware)
        {
            yield return hw;
            foreach (var sub in hw.SubHardware) yield return sub;
        }
    }
}

public sealed class FanControlException : Exception
{
    public FanControlException(string code, string message) : base(message)
    {
        Code = code;
    }

    public string Code { get; }
}
