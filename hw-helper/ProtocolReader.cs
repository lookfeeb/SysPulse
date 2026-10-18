using System.Text;

namespace SysPulse.HwHelper;

public static class ProtocolReader
{
    public static string? ReadLine(TextReader input, int limit = 16 * 1024)
    {
        var line = new StringBuilder();
        while (true)
        {
            int next = input.Read();
            if (next < 0)
            {
                if (line.Length == 0) return null;
                throw new IOException("Incomplete request frame");
            }
            if (next == '\n') return line.ToString();
            if (line.Length >= limit) throw new IOException("Request frame exceeds limit");
            line.Append((char)next);
        }
    }
}
