using System.Buffers;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace SysPulse.HwHelper;

/// Single-threaded JSON-line writer. Reuses its UTF-8 buffer across requests.
public sealed class SnapshotResponseWriter : IDisposable
{
    private readonly Stream _output;
    private readonly ArrayBufferWriter<byte> _buffer = new(4096);
    private readonly Utf8JsonWriter _json;

    public SnapshotResponseWriter(Stream output)
    {
        _output = output;
        _json = new Utf8JsonWriter(_buffer);
    }

    public void Write(long id, HwSnapshot snapshot)
    {
        _buffer.Clear();
        _json.Reset(_buffer);
        JsonSerializer.Serialize(_json, new SnapshotResponse(id, true, snapshot), HelperJsonContext.Default.SnapshotResponse);
        _json.Flush();
        _buffer.GetSpan(1)[0] = (byte)'\n';
        _buffer.Advance(1);
        _output.Write(_buffer.WrittenSpan);
        _output.Flush();
    }

    // The caller retains ownership of stdout.
    public void Dispose() => _json.Dispose();
}

internal readonly record struct SnapshotResponse(long Id, bool Ok, HwSnapshot Data);

[JsonSourceGenerationOptions(PropertyNamingPolicy = JsonKnownNamingPolicy.CamelCase,
    DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull, GenerationMode = JsonSourceGenerationMode.Serialization)]
[JsonSerializable(typeof(SnapshotResponse))]
[JsonSerializable(typeof(Request), GenerationMode = JsonSourceGenerationMode.Metadata)]
internal partial class HelperJsonContext : JsonSerializerContext;
