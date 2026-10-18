using LibreHardwareMonitor.Hardware;

namespace SysPulse.HwHelper;

public static class FanControl
{
    private static readonly HashSet<IControl> ManualControls = new(ReferenceEqualityComparer.Instance);
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

        ManualControls.Add(target); // A failing write may still have reached the controller.
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
        var attempted = 0;
        var succeeded = 0;
        var failed = 0;
        foreach (var control in ManualControls.ToArray())
        {
            attempted++;
            try
            {
                control.SetDefault();
                ManualControls.Remove(control);
                succeeded++;
            }
            catch (Exception ex)
            {
                failed++;
                Console.Error.WriteLine($"[hw-helper] reset fan control failed: {ex.Message}");
                // Keep reset_all best-effort. A single bad controller should not
                // prevent other fans from returning to firmware control.
            }
        }

        return new { attempted, succeeded, failed };
    }

    private static IControl? FindControl(ComputerHost host, string fanId) => FindControl(AllHardware(host), fanId);

    public static IControl? FindControl(IEnumerable<IHardware> hardware, string fanId)
    {
        foreach (var hw in hardware)
        {
            var sensors = FanChannels.Capture(hw, readValues: false);
            var fans = sensors.Where(s => s.Type == SensorType.Fan && s.Id == fanId).ToArray();
            if (fans.Length == 1) return FanChannels.Resolve(hw.HardwareType, sensors, fans[0])?.Control;
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
            ManualControls.Remove(control);
        }
        catch (Exception ex)
        {
            throw new FanControlException(ErrorCodes.EcWriteFail, ex.Message);
        }
    }

    private static IEnumerable<IHardware> AllHardware(ComputerHost host)
    {
        var seen = new HashSet<IHardware>(ReferenceEqualityComparer.Instance);
        var pending = new Stack<IHardware>(host.Inner.Hardware);
        while (pending.TryPop(out var hw))
        {
            if (!seen.Add(hw)) continue;
            yield return hw;
            foreach (var sub in hw.SubHardware) pending.Push(sub);
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
