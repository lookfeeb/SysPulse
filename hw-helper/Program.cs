using System.Text.Json;
using System.Text.Json.Serialization;
using SysPulse.HwHelper;

// =============================================================================
// hw-helper.exe — bridges Rust ↔ LibreHardwareMonitor via JSON-line over stdio.
//
// Protocol:
//   stdin :  one Request JSON object per line
//   stdout:  one Response or EventMessage JSON object per line
//   stderr:  free-form text logs (forwarded to tracing on Rust side)
//
// Sensor/control access is serialized. Rust owns curves and the watchdog;
// the helper tracks its own control writes for best-effort restoration on exit.
// =============================================================================

var jsonOpts = new JsonSerializerOptions
{
    PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
    DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    WriteIndented = false,
};

// Standard streams. Use raw Console.OpenStandard* to avoid locale codepage
// issues with Chinese sensor names from LHM.
using var stdin = new StreamReader(Console.OpenStandardInput(), System.Text.Encoding.UTF8);
using var stdout = new StreamWriter(Console.OpenStandardOutput(), new System.Text.UTF8Encoding(false))
{
    AutoFlush = true,
    NewLine = "\n",
};

using var host = new ComputerHost();
try
{
    host.Open();
}
catch (Exception ex)
{
    EmitFatal($"failed to open LHM: {ex}");
    return 2;
}

// Publish optional metadata as one completed cache, without delaying readiness.
var wmiCache = new WmiCache();
var metadataTask = Task.Run(() => { var cache = new WmiCache(); cache.Refresh(); return cache; });

EmitEvent("ready", new { lhmHardwareCount = host.Inner.Hardware.Count });

// Optional: react to Ctrl+C / parent termination by closing LHM cleanly.
Console.CancelKeyPress += (_, args) =>
{
    args.Cancel = true; // we'll exit naturally after Close()
    lock (host) host.Dispose();
    Environment.Exit(0);
};

string? line;
while ((line = ReadRequestLine()) != null)
{
    if (string.IsNullOrWhiteSpace(line)) continue;

    Request? req;
    try
    {
        req = JsonSerializer.Deserialize<Request>(line, jsonOpts);
    }
    catch (Exception ex)
    {
        WriteResponse(new Response
        {
            Id = -1,
            Ok = false,
            Error = new ErrorPayload { Code = ErrorCodes.InvalidParam, Message = ex.Message },
        });
        continue;
    }
    if (req is null) continue;

    if (metadataTask.IsCompletedSuccessfully) wmiCache = metadataTask.Result;
    Response response;
    lock (host) response = HandleRequest(req, host, wmiCache);
    WriteResponse(response);

    if (req.Op == "shutdown")
    {
        break;
    }
}

host.Dispose();
return 0;

string? ReadRequestLine()
{
    try { return ProtocolReader.ReadLine(stdin); }
    catch (IOException ex) { Console.Error.WriteLine($"[hw-helper] invalid input: {ex.Message}"); return null; }
}

// -----------------------------------------------------------------------------
// Local helpers
// -----------------------------------------------------------------------------

Response HandleRequest(Request req, ComputerHost h, WmiCache wmi)
{
    try
    {
        return req.Op switch
        {
            "snapshot" => Ok(req.Id, SnapshotBuilder.Build(h, wmi)),
            "set_fan" => Ok(req.Id, FanControl.SetFan(h, req.Params)),
            "reset_fan" => Ok(req.Id, FanControl.ResetFan(h, req.Params)),
            "reset_fans" => Ok(req.Id, FanControl.ResetFans(h)),
            "shutdown" => Ok(req.Id, null),
            _ => Err(req.Id, ErrorCodes.InvalidParam, $"unknown op: {req.Op}"),
        };
    }
    catch (FanControlException ex)
    {
        return Err(req.Id, ex.Code, ex.Message);
    }
    catch (Exception ex)
    {
        Console.Error.WriteLine($"[hw-helper] op={req.Op} threw: {ex}");
        return Err(req.Id, ErrorCodes.Internal, ex.Message);
    }
}

Response Ok(long id, object? data) => new() { Id = id, Ok = true, Data = data };
Response Err(long id, string code, string msg) => new()
{
    Id = id,
    Ok = false,
    Error = new ErrorPayload { Code = code, Message = msg },
};

void WriteResponse(Response r)
{
    var json = JsonSerializer.Serialize(r, jsonOpts);
    stdout.WriteLine(json);
}

void EmitEvent(string name, object? data)
{
    var msg = new EventMessage { Event = name, Data = data };
    var json = JsonSerializer.Serialize(msg, jsonOpts);
    stdout.WriteLine(json);
}

void EmitFatal(string msg)
{
    Console.Error.WriteLine($"[hw-helper] FATAL: {msg}");
    EmitEvent("fatal", new { message = msg });
}
