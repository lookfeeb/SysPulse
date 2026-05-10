using LibreHardwareMonitor.Hardware;

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
    public void Refresh()
    {
        foreach (IHardware hw in _computer.Hardware)
        {
            hw.Update();
            foreach (IHardware sub in hw.SubHardware)
            {
                sub.Update();
            }
        }
    }

    public void Dispose()
    {
        if (_opened)
        {
            try { _computer.Close(); } catch { /* ignored */ }
            _opened = false;
        }
    }
}
