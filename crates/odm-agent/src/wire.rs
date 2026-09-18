//! What the agent says, picked out of the wire JSON field by field.
//!
//! Deliberately not typed deserialization: adapters ship non-schema fields
//! and notifications freely, and one unexpected value must cost that value,
//! not the whole update. Everything here is total — a missing or odd field
//! reads as absent. Field names follow ACP v1 (`agent-client-protocol-schema`).

use serde_json::Value;

/// `initialize`'s answer, as far as the panel cares.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentInfo {
    /// `agentInfo.title`, else `name`.
    pub title: Option<String>,
    pub version: Option<String>,
    pub load_session: bool,
    /// `_meta.steering.supported`: the agent takes `_session/steering`.
    pub steering: bool,
    /// How to log in, when the agent says: (name, description).
    pub auth_methods: Vec<(String, Option<String>)>,
}

/// A piece of a message. Chunks sharing a `message_id` are one message.
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub message_id: Option<String>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolContent {
    Text(String),
    Diff { path: String, old: Option<String>, new: String },
}

/// A tool call, or an update to one: every field but the id is "if present".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub title: Option<String>,
    /// `read`, `edit`, `execute`, …
    pub kind: Option<String>,
    /// `pending`, `in_progress`, `completed`, `failed`.
    pub status: Option<String>,
    pub content: Option<Vec<ToolContent>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanEntry {
    pub content: String,
    /// `pending`, `in_progress`, `completed`.
    pub status: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub value: String,
    pub name: String,
}

/// One selector the agent offers for the session (mode, model, effort, …).
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    /// `mode`, `model`, `thought_level`, … when the agent says.
    pub category: Option<String>,
    pub current: String,
    pub choices: Vec<Choice>,
    pub(crate) kind: OptionKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum OptionKind {
    Select,
    Boolean,
    /// Synthesized from the older `modes` object; set with `session/set_mode`.
    LegacyMode,
}

impl ConfigOption {
    pub fn is_mode(&self) -> bool {
        self.category.as_deref() == Some("mode")
    }

    pub fn current_name(&self) -> Option<&str> {
        self.choices.iter().find(|c| c.value == self.current).map(|c| c.name.as_str())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PermissionOption {
    pub id: String,
    pub name: String,
    /// `allow_once`, `allow_always`, `reject_once`, `reject_always`.
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    UserChunk(Chunk),
    AgentChunk(Chunk),
    ThoughtChunk(Chunk),
    /// A new call or a change to one; fold by `id`.
    ToolCall(ToolCall),
    /// The whole plan, replacing the last.
    Plan(Vec<PlanEntry>),
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::to_owned)
}

pub(crate) fn agent_info(result: &Value) -> AgentInfo {
    let info = result.get("agentInfo").unwrap_or(&Value::Null);
    let caps = result.get("agentCapabilities").unwrap_or(&Value::Null);
    AgentInfo {
        title: str_of(info, "title").or_else(|| str_of(info, "name")),
        version: str_of(info, "version"),
        load_session: caps.get("loadSession").and_then(Value::as_bool).unwrap_or(false),
        steering: result
            .pointer("/_meta/steering/supported")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        auth_methods: result
            .get("authMethods")
            .and_then(Value::as_array)
            .map(|methods| {
                methods
                    .iter()
                    .filter_map(|m| Some((str_of(m, "name")?, str_of(m, "description"))))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// A content block as the text the transcript shows for it.
fn content_text(content: &Value) -> String {
    match content.get("type").and_then(Value::as_str) {
        Some("text") => str_of(content, "text").unwrap_or_default(),
        Some("resource") => content
            .pointer("/resource/uri")
            .and_then(Value::as_str)
            .unwrap_or("[resource]")
            .to_owned(),
        Some("resource_link") => str_of(content, "uri").unwrap_or_else(|| "[link]".to_owned()),
        Some(other) => format!("[{other}]"),
        None => String::new(),
    }
}

fn chunk(update: &Value) -> Chunk {
    Chunk {
        message_id: str_of(update, "messageId"),
        text: content_text(update.get("content").unwrap_or(&Value::Null)),
    }
}

pub(crate) fn tool_call(v: &Value) -> Option<ToolCall> {
    let content = v.get("content").and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(|item| match item.get("type").and_then(Value::as_str)? {
                "content" => Some(ToolContent::Text(content_text(item.get("content")?))),
                "diff" => Some(ToolContent::Diff {
                    path: str_of(item, "path")?,
                    old: str_of(item, "oldText"),
                    new: str_of(item, "newText").unwrap_or_default(),
                }),
                _ => None,
            })
            .collect()
    });
    Some(ToolCall {
        id: str_of(v, "toolCallId")?,
        title: str_of(v, "title"),
        kind: str_of(v, "kind"),
        status: str_of(v, "status"),
        content,
    })
}

/// A `session/update`'s payload. None = a kind this client has no use for
/// (or has never heard of): skipped, never fatal.
pub(crate) fn update(update: &Value) -> Option<Parsed> {
    Some(match update.get("sessionUpdate")?.as_str()? {
        "user_message_chunk" => Parsed::Update(Update::UserChunk(chunk(update))),
        "agent_message_chunk" => Parsed::Update(Update::AgentChunk(chunk(update))),
        "agent_thought_chunk" => Parsed::Update(Update::ThoughtChunk(chunk(update))),
        "tool_call" | "tool_call_update" => Parsed::Update(Update::ToolCall(tool_call(update)?)),
        "plan" => Parsed::Update(Update::Plan(
            update
                .get("entries")?
                .as_array()?
                .iter()
                .filter_map(|e| {
                    Some(PlanEntry {
                        content: str_of(e, "content")?,
                        status: str_of(e, "status").unwrap_or_else(|| "pending".to_owned()),
                    })
                })
                .collect(),
        )),
        "config_option_update" => Parsed::ConfigOptions(config_options(update)),
        "current_mode_update" => Parsed::CurrentMode(str_of(update, "currentModeId")?),
        "usage_update" => Parsed::Usage {
            used: update.get("used")?.as_u64()?,
            size: update.get("size")?.as_u64()?,
        },
        _ => return None,
    })
}

pub(crate) enum Parsed {
    Update(Update),
    ConfigOptions(Vec<ConfigOption>),
    CurrentMode(String),
    Usage { used: u64, size: u64 },
}

/// `configOptions` of a session response or update — plus, when none of them
/// is the mode, the older `modes` object as one more option.
pub(crate) fn config_options(v: &Value) -> Vec<ConfigOption> {
    let mut options: Vec<ConfigOption> = v
        .get("configOptions")
        .and_then(Value::as_array)
        .map(|options| options.iter().filter_map(config_option).collect())
        .unwrap_or_default();
    if !options.iter().any(ConfigOption::is_mode)
        && let Some(modes) = v.get("modes")
        && let Some(current) = str_of(modes, "currentModeId")
    {
        let choices = modes
            .get("availableModes")
            .and_then(Value::as_array)
            .map(|modes| {
                modes
                    .iter()
                    .filter_map(|m| Some(Choice { value: str_of(m, "id")?, name: str_of(m, "name")? }))
                    .collect()
            })
            .unwrap_or_default();
        options.insert(
            0,
            ConfigOption {
                id: "mode".to_owned(),
                name: "Mode".to_owned(),
                category: Some("mode".to_owned()),
                current,
                choices,
                kind: OptionKind::LegacyMode,
            },
        );
    }
    options
}

fn config_option(v: &Value) -> Option<ConfigOption> {
    let (kind, current, choices) = match v.get("type").and_then(Value::as_str)? {
        "select" => {
            let mut choices = Vec::new();
            for entry in v.get("options")?.as_array()? {
                // Grouped or not; the groups' own names are dropped.
                match entry.get("options").and_then(Value::as_array) {
                    Some(group) => choices.extend(group.iter().filter_map(choice)),
                    None => choices.extend(choice(entry)),
                }
            }
            (OptionKind::Select, str_of(v, "currentValue")?, choices)
        }
        "boolean" => {
            let choices = [("true", "On"), ("false", "Off")]
                .map(|(value, name)| Choice { value: value.to_owned(), name: name.to_owned() });
            (OptionKind::Boolean, v.get("currentValue")?.as_bool()?.to_string(), choices.into())
        }
        _ => return None,
    };
    Some(ConfigOption {
        id: str_of(v, "id")?,
        name: str_of(v, "name")?,
        category: str_of(v, "category"),
        current,
        choices,
        kind,
    })
}

fn choice(v: &Value) -> Option<Choice> {
    Some(Choice { value: str_of(v, "value")?, name: str_of(v, "name")? })
}

pub(crate) fn permission_options(params: &Value) -> Vec<PermissionOption> {
    params
        .get("options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|o| {
                    Some(PermissionOption {
                        id: str_of(o, "optionId")?,
                        name: str_of(o, "name")?,
                        kind: str_of(o, "kind").unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
