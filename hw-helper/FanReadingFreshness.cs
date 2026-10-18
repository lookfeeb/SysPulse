using System.Reflection;
using System.Runtime.CompilerServices;
using LibreHardwareMonitor.Hardware;

namespace SysPulse.HwHelper;

/// <summary>Guard LHM 0.9.4 NVIDIA reads that retain Value when a driver query fails.</summary>
public static class FanReadingFreshness
{
    private sealed class Cycle { public bool Cleared; }
    private static readonly ConditionalWeakTable<ISensor, Cycle> Cycles = new();
    private static readonly Type? LibrarySensorType = typeof(Computer).Assembly.GetType("LibreHardwareMonitor.Hardware.Sensor");
    private static readonly PropertyInfo? ValueProperty = LibrarySensorType?.GetProperty("Value", BindingFlags.Public | BindingFlags.Instance);

    public static void BeforeUpdate(IHardware hardware)
    {
        if (hardware.HardwareType != HardwareType.GpuNvidia) return;
        foreach (var sensor in hardware.Sensors)
        {
            if (!IsCooling(sensor)) continue;
            var cycle = Cycles.GetOrCreateValue(sensor);
            cycle.Cleared = false;
            try
            {
                // This exact LHM setter only clears its in-memory reading. It does
                // not write to a driver, IControl, or firmware. Fail closed if the
                // pinned library changes or the sensor is an unsupported subtype.
                if (sensor.GetType() != LibrarySensorType || ValueProperty?.SetMethod is null) continue;
                sensor.ValuesTimeWindow = TimeSpan.Zero;
                ValueProperty.SetValue(sensor, null);
                cycle.Cleared = true;
            }
            catch { /* The current cycle will publish null for this sensor. */ }
        }
    }

    public static bool CanRead(IHardware hardware, ISensor sensor) =>
        hardware.HardwareType != HardwareType.GpuNvidia || !IsCooling(sensor)
        || (Cycles.TryGetValue(sensor, out var cycle) && cycle.Cleared);

    private static bool IsCooling(ISensor sensor) => sensor.SensorType is SensorType.Fan or SensorType.Control;
}
