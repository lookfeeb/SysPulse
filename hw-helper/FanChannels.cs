using LibreHardwareMonitor.Hardware;
using System.Text.RegularExpressions;

namespace SysPulse.HwHelper;

/// <summary>One association policy for both telemetry and writes; indices are not identities.</summary>
public static class FanChannels
{
    private static readonly Regex BoardLabel = new(@"^(?:CPU(?: Optional)?|System|Chassis) Fan(?: #?\d+)?$",
        RegexOptions.IgnoreCase | RegexOptions.CultureInvariant);
    public sealed record Reading(string Name, string Id, int Index, SensorType Type, double? Value, IControl? Control);
    public sealed record ControlMatch(string SensorId, double? Pwm, IControl Control);

    public static List<Reading> Capture(IHardware hw, bool readValues = true)
    {
        var sensors = hw.Sensors;
        var readings = new List<Reading>(sensors.Length);
        foreach (var sensor in sensors)
        {
            try
            {
                double? value = null;
                if (readValues)
                {
                    try { if (sensor.ValuesTimeWindow != TimeSpan.Zero) sensor.ValuesTimeWindow = TimeSpan.Zero; }
                    catch { /* Optional history configuration. */ }
                    if (FanReadingFreshness.CanRead(hw, sensor))
                        try { value = sensor.Value; } catch { /* Preserve identity, leave the reading unknown. */ }
                }
                IControl? control = null;
                try { control = sensor.Control; } catch { /* Optional capability. */ }
                readings.Add(new(sensor.Name, sensor.Identifier.ToString(), sensor.Index, sensor.SensorType,
                    value is double v && double.IsFinite(v) ? v : null, control));
            }
            catch { /* A broken descriptor must not discard neighbouring channels. */ }
        }
        return readings;
    }

    public static ControlMatch? Resolve(HardwareType hardwareType, IReadOnlyList<Reading> sensors, Reading fan)
    {
        if (fan.Type != SensorType.Fan) return null;
        if (fan.Control is IControl attached)
        {
            var linked = sensors.Where(s => s.Type == SensorType.Control && ReferenceEquals(s.Control, attached)).ToArray();
            return new(linked.Length == 1 ? linked[0].Id : fan.Id, linked.Length == 1 ? linked[0].Value : null, attached);
        }

        // LHM's board-specific labels (e.g. B75M-D3H CPU Fan: RPM 0, PWM 2)
        // can identify a channel. Generic "Fan #1" labels and GPU ordinals cannot.
        if (hardwareType is not (HardwareType.SuperIO or HardwareType.Motherboard)) return null;
        var name = fan.Name.Trim();
        if (!IsBoardFanLabel(name)) return null;
        bool SameName(Reading s) => s.Name.Trim().Equals(name, StringComparison.OrdinalIgnoreCase);
        if (sensors.Count(s => s.Type == SensorType.Fan && SameName(s)) != 1) return null;
        var matches = sensors.Where(s => s.Type == SensorType.Control && SameName(s)).ToArray();
        return matches.Length == 1 && matches[0].Control is IControl control
            ? new(matches[0].Id, matches[0].Value, control) : null;
    }

    private static bool IsBoardFanLabel(string name) => BoardLabel.IsMatch(name);

    // No desktop-specific ceiling: high-speed server fans are valid too.
    public static double? Rpm(double? value) => value is double v && double.IsFinite(v) && v >= 0 ? v : null;
}
