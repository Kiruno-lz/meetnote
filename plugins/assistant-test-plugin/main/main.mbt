///|
/// A small, response-driven MoonBit capability consumer.
///
/// The executable never opens MeetNote data. It receives the previous Host
/// response as an argument, extracts live revision/hash/receipt fields, and
/// constructs the next typed call from those values.
fn json_at(value : Json, path : Array[String], index? : Int = 0) -> Json? {
  if index >= path.length() {
    Some(value)
  } else {
    match value {
      Object(object) =>
        match object.get(path[index]) {
          Some(next) => json_at(next, path, index=index + 1)
          None => None
        }
      _ => None
    }
  }
}

///|
fn json_string(value : Json?, fallback : String) -> String {
  match value {
    Some(String(value)) => value
    _ => fallback
  }
}

///|
fn json_number(value : Json?, fallback : String) -> String {
  match value {
    Some(Number(value, ..)) => value.to_uint64().to_string()
    _ => fallback
  }
}

///|
fn json_array_string(
  value : Json?,
  path : Array[String],
  index : Int,
  field : String,
  fallback : String,
) -> String {
  match json_at(value.unwrap_or(null), path) {
    Some(Array(values)) =>
      match values.get(index) {
        Some(Object(object)) => json_string(object.get(field), fallback)
        Some(String(value)) => value
        _ => fallback
      }
    _ => fallback
  }
}

///|
fn environment_response(name : String) -> Json? {
  match @env.get_env_var(name) {
    Some(response) => Some(@json.parse(response)) catch { _ => None }
    None => None
  }
}

///|
fn previous_response() -> Json? {
  environment_response("MEETNOTE_HOST_RESPONSE")
}

///|
fn workflow_call(previous : Json?) -> String {
  match previous {
    None =>
      // Intentionally omit the source hash. The real Host error tells the
      // plugin that this external write needs a read-derived proof.
      "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"update_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"expectedRevision\":0,\"patch\":{\"title\":\"Codex Host task\"}}}"
    Some(response) => {
      let ok = match json_at(response, ["ok"]) {
        Some(True) => true
        _ => false
      }
      let error_code = json_string(json_at(response, ["error", "code"]), "")
      if !ok && error_code == "TOOL_ARGUMENTS_INVALID" {
        // The error is followed by a live read, not a hardcoded fixture.
        "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"read_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"projection\":\"metadata\"}}"
      } else if ok &&
        json_at(response, ["result", "payload", "metadata", "markdownSha256"])
        is Some(_) {
        let title = json_string(
          json_at(response, ["result", "payload", "metadata", "title"]),
          "",
        )
        if title == "Codex Host task" {
          let mutation = environment_response("MEETNOTE_HOST_HISTORY").unwrap_or(
            null,
          )
          let undo_token = json_string(
            json_at(mutation, ["result", "payload", "undoToken"]),
            "",
          )
          "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"undo_mutation\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"undoToken\":\"" +
          undo_token +
          "\"}}"
        } else if title == "Fixture" {
          let revision = json_number(
            json_at(response, ["result", "payload", "metadata", "revision"]),
            "0",
          )
          let hash = json_string(
            json_at(response, [
              "result", "payload", "metadata", "markdownSha256",
            ]),
            "",
          )
          "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"update_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"expectedRevision\":" +
          revision +
          ",\"sourceMarkdownSha256\":\"" +
          hash +
          "\",\"patch\":{\"title\":\"Codex Host task\"}}}"
        } else {
          "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"read_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"projection\":\"metadata\"}}"
        }
      } else if ok && json_at(response, ["result", "type"]) is Some(_) {
        // Read back before using the receipt for the compensating control.
        "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"read_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"projection\":\"metadata\"}}"
      } else {
        "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"read_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"projection\":\"metadata\"}}"
      }
    }
  }
}

///|
fn operation_call(operation : String, previous : Json?) -> String {
  match operation {
    "list" =>
      "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"list_documents\",\"authorization\":{\"allDocuments\":true},\"arguments\":{}}"
    "guarded-update" =>
      match previous {
        None =>
          "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"read_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"projection\":\"metadata\"}}"
        Some(response) => {
          let result_type = json_string(
            json_at(response, ["result", "type"]),
            "",
          )
          if result_type == "pending" {
            let confirmation_id = json_string(
              json_at(response, ["result", "payload", "confirmationId"]),
              "",
            )
            "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"confirm_mutation\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"confirmationId\":\"" +
            confirmation_id +
            "\"}}"
          } else {
            let revision = json_number(
              json_at(response, ["result", "payload", "metadata", "revision"]),
              "0",
            )
            let hash = json_string(
              json_at(response, [
                "result", "payload", "metadata", "markdownSha256",
              ]),
              "",
            )
            "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"update_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"expectedRevision\":" +
            revision +
            ",\"sourceMarkdownSha256\":\"" +
            hash +
            "\",\"patch\":{\"title\":\"Guarded Host task\"}}}"
          }
        }
      }
    "create" =>
      "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"create_document\",\"authorization\":{\"allDocuments\":true},\"arguments\":{\"title\":\"plugin-created\",\"summary\":null,\"markdown\":\"created by plugin\\n\",\"source\":{\"type\":\"no_audio\"},\"parentDocumentId\":null,\"expectedParentRevision\":null,\"sourceMarkdownSha256\":null}}"
    "semantic" =>
      match previous {
        None =>
          "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"read_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"projection\":\"complete\"}}"
        Some(response) => {
          let revision = json_number(
            json_at(response, ["result", "payload", "metadata", "revision"]),
            "0",
          )
          let hash = json_string(
            json_at(response, [
              "result", "payload", "metadata", "markdownSha256",
            ]),
            "",
          )
          let start = json_array_string(
            json_at(response, ["result", "payload", "transcript"]),
            [],
            0,
            "id",
            "",
          )
          let end = json_array_string(
            json_at(response, ["result", "payload", "transcript"]),
            [],
            1,
            "id",
            start,
          )
          "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"create_semantic_children\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"parentDocumentId\":\"document-1\",\"expectedParentRevision\":" +
          revision +
          ",\"sourceMarkdownSha256\":\"" +
          hash +
          "\",\"mode\":\"partition\",\"children\":[{\"startSegmentId\":\"" +
          start +
          "\",\"endSegmentId\":\"" +
          end +
          "\",\"title\":\"semantic child\",\"summary\":null,\"evidenceSegmentIds\":[\"" +
          start +
          "\",\"" +
          end +
          "\"]}]}}"
        }
      }
    "move" =>
      match previous {
        None =>
          "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"read_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"projection\":\"metadata\"}}"
        Some(response) => {
          let revision = json_number(
            json_at(response, ["result", "payload", "metadata", "revision"]),
            "0",
          )
          "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"move_document\",\"authorization\":{\"allDocuments\":true},\"arguments\":{\"documentId\":\"document-1\",\"expectedRevision\":" +
          revision +
          ",\"newParentDocumentId\":null,\"placement\":\"last\",\"targetSiblingDocumentId\":null}}"
        }
      }
    "confirm" => guarded_control_call("confirm_mutation", previous)
    "reject" => guarded_control_call("reject_mutation", previous)
    "settings-patch" => {
      let prior = previous.unwrap_or(null)
      if json_at(prior, ["result", "settings"]) is Some(_) {
        "{\"capabilityId\":\"settings.app\",\"capabilityVersion\":1,\"operation\":\"read\",\"authorization\":{\"settingsFields\":[\"assistant.writePolicy\"]},\"arguments\":{\"section\":\"writePolicy\"}}"
      } else {
        let revision = json_string(json_at(prior, ["result", "revision"]), "")
        if revision == "" {
          "{\"capabilityId\":\"settings.app\",\"capabilityVersion\":1,\"operation\":\"read\",\"authorization\":{\"settingsFields\":[\"assistant.writePolicy\"]},\"arguments\":{\"section\":\"writePolicy\"}}"
        } else {
          "{\"capabilityId\":\"settings.app\",\"capabilityVersion\":1,\"operation\":\"patch\",\"authorization\":{\"settingsFields\":[\"assistant.writePolicy\"]},\"arguments\":{\"expectedRevision\":\"" +
          revision +
          "\",\"patch\":{\"maxToolRounds\":9}}}"
        }
      }
    }
    "stale-revision" =>
      match previous {
        None =>
          "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"read_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"projection\":\"metadata\"}}"
        Some(response) => {
          let revision = json_number(
            json_at(response, ["result", "payload", "metadata", "revision"]),
            "0",
          )
          let hash = json_string(
            json_at(response, [
              "result", "payload", "metadata", "markdownSha256",
            ]),
            "",
          )
          let stale = if revision == "0" { "0" } else { "0" }
          "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"update_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"expectedRevision\":" +
          stale +
          ",\"sourceMarkdownSha256\":\"" +
          hash +
          "\",\"patch\":{\"title\":\"must not write\"}}}"
        }
      }
    "readonly" =>
      "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"update_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"expectedRevision\":0,\"sourceMarkdownSha256\":\"ignored\",\"patch\":{\"processing\":{}}}}"
    "unknown-field" =>
      "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"read_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"projection\":\"metadata\",\"unexpected\":true}}"
    "version-error" =>
      "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":99,\"operation\":\"read_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"projection\":\"metadata\"}}"
    "settings-credentials" =>
      "{\"capabilityId\":\"settings.app\",\"capabilityVersion\":1,\"operation\":\"read\",\"authorization\":{\"settingsFields\":[\"assistant.credentials\"]},\"arguments\":{\"section\":\"credentials\"}}"
    "settings-connections" =>
      settings_section_call("assistant.connections", "connections")
    "settings-targets" =>
      settings_section_call("assistant.connectionTargets", "connectionTargets")
    "settings-policy" =>
      settings_section_call("assistant.writePolicy", "writePolicy")
    "settings-snapshot" =>
      "{\"capabilityId\":\"settings.app\",\"capabilityVersion\":1,\"operation\":\"snapshot\",\"authorization\":{\"settingsFields\":[\"assistant.connections\",\"assistant.connectionTargets\",\"assistant.credentials\",\"assistant.writePolicy\"]},\"arguments\":{}}"
    "audio-invalid-job" =>
      "{\"capabilityId\":\"audio.processing\",\"capabilityVersion\":1,\"operation\":\"query\",\"authorization\":{\"projectIds\":[\"document-1\"]},\"arguments\":{\"projectId\":\"document-1\",\"jobId\":999999}}"
    "audio-probe" => audio_call("probe", previous)
    "audio-start" => audio_call("start", previous)
    "audio-query" => audio_call("query", previous)
    "audio-cancel" => audio_call("cancel", previous)
    "audio-result" => audio_call("result", previous)
    "audio-success" => audio_success_call(previous)
    "audio-cancel-flow" => audio_cancel_flow_call(previous)
    _ => workflow_call(previous)
  }
}

///|
fn settings_section_call(field : String, section : String) -> String {
  "{\"capabilityId\":\"settings.app\",\"capabilityVersion\":1,\"operation\":\"read\",\"authorization\":{\"settingsFields\":[\"" +
  field +
  "\"]},\"arguments\":{\"section\":\"" +
  section +
  "\"}}"
}

///|
fn guarded_control_call(control : String, previous : Json?) -> String {
  match previous {
    None =>
      "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"read_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"projection\":\"metadata\"}}"
    Some(response) => {
      let result_type = json_string(json_at(response, ["result", "type"]), "")
      if result_type == "pending" {
        let confirmation_id = json_string(
          json_at(response, ["result", "payload", "confirmationId"]),
          "",
        )
        "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"" +
        control +
        "\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"confirmationId\":\"" +
        confirmation_id +
        "\"}}"
      } else {
        let revision = json_number(
          json_at(response, ["result", "payload", "metadata", "revision"]),
          "0",
        )
        let hash = json_string(
          json_at(response, ["result", "payload", "metadata", "markdownSha256"]),
          "",
        )
        "{\"capabilityId\":\"workspace.documents\",\"capabilityVersion\":1,\"operation\":\"update_document\",\"authorization\":{\"documentIds\":[\"document-1\"]},\"arguments\":{\"documentId\":\"document-1\",\"expectedRevision\":" +
        revision +
        ",\"sourceMarkdownSha256\":\"" +
        hash +
        "\",\"patch\":{\"title\":\"Plugin guarded task\"}}}"
      }
    }
  }
}

///|
fn audio_model_pair() -> String {
  "\"asrModel\":{\"backendId\":\"fake-asr\",\"modelId\":\"fake\",\"revision\":null,\"artifactPath\":\"fixture-model\",\"artifacts\":[]},\"diarizationModel\":{\"backendId\":\"fake-diarization\",\"modelId\":\"fake\",\"revision\":null,\"artifactPath\":\"fixture-model\",\"artifacts\":[]}"
}

///|
fn audio_call(kind : String, previous : Json?) -> String {
  match kind {
    "probe" =>
      "{\"capabilityId\":\"audio.processing\",\"capabilityVersion\":1,\"operation\":\"probe\",\"authorization\":{\"projectIds\":[\"document-1\"]},\"arguments\":{\"projectId\":\"document-1\"," +
      audio_model_pair() +
      "}}"
    "start" =>
      "{\"capabilityId\":\"audio.processing\",\"capabilityVersion\":1,\"operation\":\"start\",\"authorization\":{\"projectIds\":[\"document-1\"]},\"arguments\":{\"projectId\":\"document-1\"," +
      audio_model_pair() +
      "}}"
    "query" => {
      let job_id = json_number(
        json_at(previous.unwrap_or(null), ["result", "asrJobId"]),
        "0",
      )
      "{\"capabilityId\":\"audio.processing\",\"capabilityVersion\":1,\"operation\":\"query\",\"authorization\":{\"projectIds\":[\"document-1\"]},\"arguments\":{\"projectId\":\"document-1\",\"jobId\":" +
      job_id +
      "}}"
    }
    "cancel" => audio_handle_call("cancel", previous)
    "result" => audio_handle_call("result", previous)
    _ => audio_call("probe", previous)
  }
}

///|
fn audio_handle_call(kind : String, previous : Json?) -> String {
  let prior = previous.unwrap_or(null)
  let history = environment_response("MEETNOTE_HOST_HISTORY").unwrap_or(null)
  let asr = json_number(
    first_json(
      json_at(prior, ["result", "asrJobId"]),
      json_at(history, ["result", "asrJobId"]),
    ),
    "0",
  )
  let diarization = json_number(
    first_json(
      json_at(prior, ["result", "diarizationJobId"]),
      json_at(history, ["result", "diarizationJobId"]),
    ),
    "1",
  )
  "{\"capabilityId\":\"audio.processing\",\"capabilityVersion\":1,\"operation\":\"" +
  kind +
  "\",\"authorization\":{\"projectIds\":[\"document-1\"]},\"arguments\":{\"projectId\":\"document-1\",\"asrJobId\":" +
  asr +
  ",\"diarizationJobId\":" +
  diarization +
  "}}"
}

///|
fn first_json(primary : Json?, fallback : Json?) -> Json? {
  match primary {
    Some(value) => Some(value)
    None => fallback
  }
}

///|
fn audio_success_call(previous : Json?) -> String {
  match previous {
    None => audio_call("start", None)
    Some(response) =>
      if json_at(response, ["result", "asrJobId"]) is Some(_) {
        audio_call("query", Some(response))
      } else if json_at(response, ["result", "state"]) is Some(_) {
        let state = json_string(json_at(response, ["result", "state"]), "")
        if state == "Completed" {
          audio_handle_call("result", Some(response))
        } else {
          audio_call(
            "query",
            Some(environment_response("MEETNOTE_HOST_HISTORY").unwrap_or(null)),
          )
        }
      } else {
        audio_call("start", Some(response))
      }
  }
}

///|
fn audio_cancel_flow_call(previous : Json?) -> String {
  match previous {
    None => audio_call("start", None)
    Some(response) =>
      if json_at(response, ["result", "asrJobId"]) is Some(_) {
        audio_handle_call("cancel", Some(response))
      } else {
        audio_call(
          "query",
          Some(environment_response("MEETNOTE_HOST_HISTORY").unwrap_or(null)),
        )
      }
  }
}

///|
fn main {
  let operation = @env.get_env_var("MEETNOTE_PLUGIN_OPERATION").unwrap_or(
    "workflow",
  )
  let previous = previous_response()
  println(
    "{\"type\":\"call\",\"pluginId\":\"meetnote.assistant-test\",\"version\":1,\"call\":" +
    operation_call(operation, previous) +
    "}",
  )
}
