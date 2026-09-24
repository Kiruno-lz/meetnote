/// Decode typed capability arguments. They are always JSON objects: serde would
/// otherwise accept a JSON array positionally for the same struct. Each caller
/// maps the message to its owning service's error code.
pub(crate) fn decode_object_arguments<T: serde::de::DeserializeOwned>(
    arguments: &serde_json::Value,
) -> Result<T, String> {
    if !arguments.is_object() {
        return Err("arguments must be a JSON object".into());
    }
    serde_json::from_value(arguments.clone()).map_err(|error| error.to_string())
}
