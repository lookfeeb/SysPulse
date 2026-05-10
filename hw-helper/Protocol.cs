using System.Text.Json.Serialization;

namespace SysPulse.HwHelper;

// ---------------------------------------------------------------------------
// Wire format. Fields are camelCase via JsonSerializerOptions.PropertyNamingPolicy.
// ---------------------------------------------------------------------------

public sealed class Request
{
    [JsonPropertyName("id")] public long Id { get; set; }
    [JsonPropertyName("op")] public string Op { get; set; } = "";
    [JsonPropertyName("params")] public RequestParams? Params { get; set; }
}

public sealed class RequestParams
{
    [JsonPropertyName("fanId")] public string? FanId { get; set; }
    [JsonPropertyName("mode")] public string? Mode { get; set; } // bios | manual | curve
    [JsonPropertyName("pwm")] public double? Pwm { get; set; }   // 0..100
}

public sealed class Response
{
    [JsonPropertyName("id")] public long Id { get; set; }
    [JsonPropertyName("ok")] public bool Ok { get; set; }
    [JsonPropertyName("data")] public object? Data { get; set; }
    [JsonPropertyName("error")] public ErrorPayload? Error { get; set; }
}

public sealed class EventMessage
{
    [JsonPropertyName("event")] public string Event { get; set; } = "";
    [JsonPropertyName("data")] public object? Data { get; set; }
}

public sealed class ErrorPayload
{
    [JsonPropertyName("code")] public string Code { get; set; } = "";
    [JsonPropertyName("message")] public string Message { get; set; } = "";
}

public static class ErrorCodes
{
    public const string LhmNotInit  = "LHM_NOT_INIT";
    public const string LhmTimeout  = "LHM_TIMEOUT";
    public const string InvalidParam= "INVALID_PARAM";
    public const string Unsupported = "UNSUPPORTED";
    public const string EcWriteFail = "EC_WRITE_FAIL";
    public const string Internal    = "INTERNAL";
}
