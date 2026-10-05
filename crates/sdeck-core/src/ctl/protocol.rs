//! The control protocol: one JSON object per line, each way.
//! Request `{"token":..,"id":N,"method":"ping","params":{}}`; response `{"id":N,"ok":true,"result":..}`
//! or `{"id":N,"ok":false,"error":"..."}`.

use serde_json::{json, Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct CtlRequest {
    pub token: String,
    pub id: u64,
    pub method: String,
    /// Always an object; `{}` when the line had none.
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CtlResponse {
    pub id: u64,
    /// `Ok(result)` or `Err(error message)`.
    pub outcome: Result<Value, String>,
}

impl CtlRequest {
    pub fn new(token: &str, id: u64, method: &str, params: Value) -> Self {
        Self {
            token: token.to_string(),
            id,
            method: method.to_string(),
            params,
        }
    }

    pub fn to_line(&self) -> String {
        json!({"token": self.token, "id": self.id, "method": self.method, "params": self.params}).to_string()
    }

    /// `Err` holds the id the line carried (0 if none) and why it was refused, ready to answer with.
    pub fn parse(line: &str) -> Result<Self, CtlResponse> {
        let obj = match serde_json::from_str::<Value>(line) {
            Ok(Value::Object(obj)) => obj,
            _ => return Err(CtlResponse::err(0, "request is not a JSON object")),
        };
        let id = obj.get("id").and_then(Value::as_u64).unwrap_or(0);
        let text = |key: &str| obj.get(key).and_then(Value::as_str).map(str::to_string);
        let (Some(token), Some(method)) = (text("token"), text("method")) else {
            return Err(CtlResponse::err(id, "request needs a token and a method"));
        };
        let params = match obj.get("params") {
            None | Some(Value::Null) => Value::Object(Map::new()),
            Some(p @ Value::Object(_)) => p.clone(),
            Some(_) => return Err(CtlResponse::err(id, "params must be an object")),
        };
        Ok(Self {
            token,
            id,
            method,
            params,
        })
    }
}

impl CtlResponse {
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            id,
            outcome: Ok(result),
        }
    }

    pub fn err(id: u64, error: impl Into<String>) -> Self {
        Self {
            id,
            outcome: Err(error.into()),
        }
    }

    pub fn to_line(&self) -> String {
        match &self.outcome {
            Ok(result) => json!({"id": self.id, "ok": true, "result": result}),
            Err(error) => json!({"id": self.id, "ok": false, "error": error}),
        }
        .to_string()
    }

    pub fn parse(line: &str) -> Option<Self> {
        let Value::Object(obj) = serde_json::from_str::<Value>(line).ok()? else {
            return None;
        };
        let id = obj.get("id").and_then(Value::as_u64)?;
        let outcome = if obj.get("ok").and_then(Value::as_bool)? {
            Ok(obj.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(obj
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string())
        };
        Some(Self { id, outcome })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_and_defaults_params() {
        let req = CtlRequest::new("t", 7, "ping", json!({"a": 1}));
        assert_eq!(CtlRequest::parse(&req.to_line()).unwrap(), req);
        let bare = CtlRequest::parse(r#"{"token":"t","id":1,"method":"ping"}"#).unwrap();
        assert_eq!(bare.params, json!({}));
    }

    #[test]
    fn bad_requests_answer_with_their_id() {
        assert_eq!(CtlRequest::parse("nope").unwrap_err().id, 0);
        let missing = CtlRequest::parse(r#"{"id":4,"method":"ping"}"#).unwrap_err();
        assert_eq!(missing.id, 4);
        assert!(missing.outcome.is_err());
        assert!(CtlRequest::parse(r#"{"token":"t","id":5,"method":"ping","params":[1]}"#).is_err());
    }

    #[test]
    fn response_round_trips_both_outcomes() {
        let ok = CtlResponse::ok(3, json!({"pid": 9}));
        assert_eq!(CtlResponse::parse(&ok.to_line()), Some(ok));
        let err = CtlResponse::err(4, "bad token");
        assert_eq!(err.to_line(), r#"{"id":4,"ok":false,"error":"bad token"}"#);
        assert_eq!(CtlResponse::parse(&err.to_line()), Some(err));
        assert_eq!(CtlResponse::parse("[]"), None);
    }
}
