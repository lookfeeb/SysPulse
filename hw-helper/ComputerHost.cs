using LibreHardwareMonitor.Hardware;
using System.Diagnostics;
using System.Runtime.CompilerServices;

namespace SysPulse.HwHelper;

/// <summary>
/// Wraps the LHM <see cref="Computer"/> singleton: opens at startup, exposes
/// helpers to enumerate hardware, and ensures we Close() on disposal so any
/// kernel resources (WinRing0 driver, control writes) are released.
/// </summary>
public sealed class ComputerHost : IDisposable
{
    private readonly Computer _computer;
    private bool _opened;
    private sealed class RetryState { public int Failures; public long FailedAt; }
    private static readonly ConditionalWeakTable<IHardware, RetryState> FailedDevices = new();

    public ComputerHost()
    {
        _computer = new Computer
        {
            IsCpuEnabled = true,
            IsGpuEnabled = true,
            IsMemoryEnabled = true,
            IsMotherboardEnabled = true,
            IsControllerEnabled = true,
            IsStorageEnabled = true,
            IsNetworkEnabled = false, // network is handled by Rust side
            IsBatteryEnabled = false,
            IsPsuEnabled = false,
        };
    }

    public Computer Inner => _computer;

    public void Open()
    {
        if (_opened) return;
        _computer.Open();
        _opened = true;
    }

    /// <summary>
    /// Force LHM to refresh all hardware sensor values. Call before reading.
    /// </summary>
    public IReadOnlyList<IHardware> Refresh() => Refresh(_computer.Hardware);

    // Only successfully refreshed devices may contribute readings. A failed
    // Update can leave last-cycle values in LHM, which must not look fresh.
    public static IReadOnlyList<IHardware> Refresh(IEnumerable<IHardware> roots)
    {
        var refreshed = new List<IHardware>();
        var seen = new HashSet<IHardware>(ReferenceEqualityComparer.Instance);
        void Visit(IHardware hw, int depth)
        {
            if (depth > 16 || !seen.Add(hw)) return;
            var coolingDown = FailedDevices.TryGetValue(hw, out var retry) &&
                Stopwatch.GetElapsedTime(retry.FailedAt).TotalSeconds < Math.Min(30, 1 << Math.Min(retry.Failures, 5));
            if (!coolingDown) try
            {
                FanReadingFreshness.BeforeUpdate(hw);
                hw.Update();
                FailedDevices.Remove(hw);
                refreshed.Add(hw);
            }
            catch (Exception ex)
            {
                retry = FailedDevices.GetOrCreateValue(hw);
                retry.Failures = Math.Min(5, retry.Failures + 1);
                retry.FailedAt = Stopwatch.GetTimestamp();
                Console.Error.WriteLine($"[hw-helper] device update failed: {ex.Message}");
            }
            try
            {
                foreach (var sub in hw.SubHardware) Visit(sub, depth + 1);
            }
            catch (Exception ex) { Console.Error.WriteLine($"[hw-helper] device enumeration failed: {ex.Message}"); }
        }
        foreach (var hw in roots) Visit(hw, 0);
        return refreshed;
    }

    public void Dispose()
    {
        lock (this)
        {
            if (_opened)
            {
                try { FanControl.ResetFans(this); } catch { /* Close still must run. */ }
                try { _computer.Close(); } catch { /* ignored */ }
                _opened = false;
            }
        }
    }
}
