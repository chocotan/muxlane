use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ThreadItem {
    Message(Message),
    Content(ContentItem),
    Thought(Thought),
    Tool(Tool),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    /// ACP identity is separate from the stable timeline identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_id: Option<String>,
    pub role: MessageRole,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContentItem {
    pub id: String,
    pub role: MessageRole,
    pub content: ToolContent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Thought {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_id: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tool {
    pub id: String,
    pub title: String,
    pub kind: ToolKind,
    pub state: ToolState,
    pub content: Vec<ToolContent>,
    pub locations: Vec<ToolLocation>,
    pub raw_input: Option<Value>,
    pub raw_output: Option<Value>,
    pub subagent_session_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    #[default]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolContent {
    Text(String),
    Image {
        data: String,
        mime_type: String,
    },
    Audio {
        data: String,
        mime_type: String,
    },
    Resource {
        uri: String,
        text: Option<String>,
    },
    ResourceLink {
        name: String,
        uri: String,
    },
    Diff {
        path: String,
        old_text: Option<String>,
        new_text: String,
    },
    Terminal {
        id: String,
    },
    Unknown(Value),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolLocation {
    pub path: String,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolState {
    #[default]
    Pending,
    Running,
    Completed,
    Failed,
    Rejected,
    Canceled,
    Unknown(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Plan {
    pub entries: Vec<PlanEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanEntry {
    pub content: String,
    pub priority: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    pub used: u64,
    pub size: u64,
    pub cost: Option<Cost>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cost {
    pub amount: f64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AvailableCommand {
    pub name: String,
    pub description: String,
    pub input_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionMode {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Modes {
    pub current: String,
    pub available: Vec<SessionMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConfigValue {
    Select(String),
    Boolean(bool),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConfigKind {
    Select {
        current: String,
        choices: Vec<ConfigChoice>,
    },
    Boolean {
        current: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigChoice {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub kind: ConfigKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ThreadDelta {
    MessageChunk {
        id: Option<String>,
        role: MessageRole,
        text: String,
    },
    ThoughtChunk {
        id: Option<String>,
        text: String,
    },
    ContentBlock {
        id: Option<String>,
        role: MessageRole,
        content: ToolContent,
    },
    ToolUpsert {
        id: String,
        title: Option<String>,
        kind: Option<ToolKind>,
        state: Option<ToolState>,
        content: Option<Vec<ToolContent>>,
        locations: Option<Vec<ToolLocation>>,
        raw_input: Option<Value>,
        raw_output: Option<Value>,
        subagent_session_id: Option<String>,
    },
    Plan(Plan),
    Usage(Usage),
    AvailableCommands(Vec<AvailableCommand>),
    Modes(Modes),
    ConfigOptions(Vec<ConfigOption>),
    SessionInfo(Value),
    Unknown(Value),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ThreadSnapshot {
    pub revision: u64,
    pub items: Vec<ThreadItem>,
    pub plan: Option<Plan>,
    pub usage: Option<Usage>,
    pub available_commands: Vec<AvailableCommand>,
    pub modes: Option<Modes>,
    pub config_options: Vec<ConfigOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadChange {
    Item { index: usize, appended: bool },
    Plan,
    Usage,
    AvailableCommands,
    Modes,
    ConfigOptions,
    Metadata,
    None,
}

enum ReplayChunk {
    Append,
    Skip,
    Replace(String),
}

enum LocalUserEcho {
    NoMatch,
    Pending,
    Completed,
}

#[derive(Debug, Default)]
pub struct ThreadReducer {
    snapshot: ThreadSnapshot,
    next_local_id: u64,
    restored_messages: std::collections::HashMap<(String, MessageRole), String>,
    replay_progress: std::collections::HashMap<(String, MessageRole), String>,
    user_echo_progress: std::collections::HashMap<String, String>,
    content_counts: std::collections::HashMap<String, u64>,
}

impl ThreadReducer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_snapshot(mut snapshot: ThreadSnapshot) -> Self {
        // Older reducers split same-ID text after a thought with that ID. Repair
        // only adjacent, same-role, same-ID fragments; never erase a boundary.
        let mut items: Vec<ThreadItem> = Vec::with_capacity(snapshot.items.len());
        for item in std::mem::take(&mut snapshot.items) {
            match (items.last_mut(), &item) {
                (Some(ThreadItem::Message(previous)), ThreadItem::Message(message))
                    if previous.id == message.id && previous.role == message.role =>
                {
                    previous.text.push_str(&message.text);
                }
                (Some(ThreadItem::Thought(previous)), ThreadItem::Thought(thought))
                    if previous.id == thought.id =>
                {
                    previous.text.push_str(&thought.text);
                }
                _ => items.push(item),
            }
        }
        snapshot.items = items;
        let mut next_local_id = snapshot.revision;
        let mut occupied: std::collections::HashSet<String> = snapshot
            .items
            .iter()
            .map(|item| match item {
                ThreadItem::Message(message) => message.id.clone(),
                ThreadItem::Thought(thought) => thought.id.clone(),
                ThreadItem::Content(content) => content.id.clone(),
                ThreadItem::Tool(tool) => tool.id.clone(),
            })
            .collect();
        let mut seen = std::collections::HashSet::new();
        for item in &mut snapshot.items {
            let (id, alias, kind) = match item {
                ThreadItem::Message(message) => {
                    (&mut message.id, &mut message.protocol_id, "message")
                }
                ThreadItem::Thought(thought) => {
                    (&mut thought.id, &mut thought.protocol_id, "thought")
                }
                _ => continue,
            };
            if !seen.insert(id.clone()) {
                let original = protocol_id(id, alias.as_deref()).map(str::to_string);
                loop {
                    next_local_id = next_local_id.saturating_add(1);
                    let replacement = format!("local-{kind}-{next_local_id}");
                    if occupied.insert(replacement.clone()) {
                        *id = replacement;
                        *alias = original;
                        break;
                    }
                }
            }
        }
        let restored_messages = snapshot
            .items
            .iter()
            .filter_map(|item| match item {
                ThreadItem::Message(message) => {
                    Some(((message.id.clone(), message.role), message.text.clone()))
                }
                _ => None,
            })
            .collect();
        let mut content_counts = std::collections::HashMap::new();
        for item in &snapshot.items {
            let ThreadItem::Content(content) = item else {
                continue;
            };
            let Some((base, count)) = content.id.rsplit_once(":content:") else {
                continue;
            };
            let Ok(count) = count.parse::<u64>() else {
                continue;
            };
            let current = content_counts.entry(base.to_string()).or_insert(0);
            *current = (*current).max(count);
        }
        Self {
            snapshot,
            next_local_id,
            restored_messages,
            replay_progress: std::collections::HashMap::new(),
            user_echo_progress: std::collections::HashMap::new(),
            content_counts,
        }
    }

    pub fn snapshot(&self) -> &ThreadSnapshot {
        &self.snapshot
    }

    pub fn into_snapshot(self) -> ThreadSnapshot {
        self.snapshot
    }

    pub fn apply(&mut self, delta: ThreadDelta) -> ThreadChange {
        let change = match delta {
            ThreadDelta::MessageChunk { id, role, text } => {
                if id.is_some() && role == MessageRole::User {
                    let protocol_id = id.as_deref().unwrap();
                    match self.consume_local_user_echo(protocol_id, &text) {
                        LocalUserEcho::Completed => ThreadChange::Item {
                            index: self.snapshot.items.len().saturating_sub(1),
                            appended: false,
                        },
                        LocalUserEcho::Pending => ThreadChange::None,
                        LocalUserEcho::NoMatch => {
                            let resolved = self.message_id(id.clone(), Some(role));
                            self.apply_message_chunk(resolved, id, role, text)
                        }
                    }
                } else {
                    let resolved = self.message_id(id.clone(), Some(role));
                    self.apply_message_chunk(resolved, id, role, text)
                }
            }
            ThreadDelta::ThoughtChunk { id, text } => {
                let protocol_id = id.clone();
                let id = self.message_id(id, None);
                let index = self.snapshot.items.iter().position(|item| {
                    matches!(item,
                        ThreadItem::Thought(thought) if thought.id == id)
                });
                if let Some(ThreadItem::Thought(thought)) =
                    index.map(|index| &mut self.snapshot.items[index])
                {
                    thought.text.push_str(&text);
                } else {
                    self.snapshot.items.push(ThreadItem::Thought(Thought {
                        id,
                        protocol_id,
                        text,
                    }));
                }
                ThreadChange::Item {
                    index: index.unwrap_or(self.snapshot.items.len() - 1),
                    appended: index.is_none(),
                }
            }
            ThreadDelta::ContentBlock { id, role, content } => {
                let base = id.unwrap_or_else(|| "local-message".into());
                let count = self.content_counts.entry(base.clone()).or_default();
                *count = count.saturating_add(1);
                let id = format!("{base}:content:{count}");
                let index = self.snapshot.items.iter().position(|item| {
                    matches!(item,
                        ThreadItem::Content(content) if content.id == id)
                });
                if let Some(ThreadItem::Content(existing)) =
                    index.map(|index| &mut self.snapshot.items[index])
                {
                    existing.role = role;
                    existing.content = content;
                } else {
                    self.snapshot.items.push(ThreadItem::Content(ContentItem {
                        id,
                        role,
                        content,
                    }));
                }
                ThreadChange::Item {
                    index: index.unwrap_or(self.snapshot.items.len() - 1),
                    appended: index.is_none(),
                }
            }
            ThreadDelta::ToolUpsert {
                id,
                title,
                kind,
                state,
                content,
                locations,
                raw_input,
                raw_output,
                subagent_session_id,
            } => {
                let index = self.snapshot.items.iter().position(|item| {
                    matches!(item,
                        ThreadItem::Tool(tool) if tool.id == id)
                });
                if let Some(ThreadItem::Tool(tool)) =
                    index.map(|index| &mut self.snapshot.items[index])
                {
                    if let Some(title) = title.filter(|title| !title.is_empty()) {
                        tool.title = title;
                    }
                    if let Some(state) = state {
                        tool.state = state;
                    }
                    if let Some(kind) = kind {
                        tool.kind = kind;
                    }
                    if let Some(content) = content {
                        tool.content = content;
                    }
                    if let Some(locations) = locations {
                        tool.locations = locations;
                    }
                    if raw_input.is_some() {
                        tool.raw_input = raw_input;
                    }
                    if raw_output.is_some() {
                        tool.raw_output = raw_output;
                    }
                    if subagent_session_id.is_some() {
                        tool.subagent_session_id = subagent_session_id;
                    }
                } else {
                    self.snapshot.items.push(ThreadItem::Tool(Tool {
                        id,
                        title: title.unwrap_or_default(),
                        kind: kind.unwrap_or_default(),
                        state: state.unwrap_or_default(),
                        content: content.unwrap_or_default(),
                        locations: locations.unwrap_or_default(),
                        raw_input,
                        raw_output,
                        subagent_session_id,
                    }));
                }
                ThreadChange::Item {
                    index: index.unwrap_or(self.snapshot.items.len() - 1),
                    appended: index.is_none(),
                }
            }
            ThreadDelta::Plan(plan) => {
                self.snapshot.plan = Some(plan);
                ThreadChange::Plan
            }
            ThreadDelta::Usage(usage) => {
                self.snapshot.usage = Some(usage);
                ThreadChange::Usage
            }
            ThreadDelta::AvailableCommands(commands) => {
                self.snapshot.available_commands = commands;
                ThreadChange::AvailableCommands
            }
            ThreadDelta::Modes(modes) => {
                if modes.available.is_empty() {
                    if let Some(existing) = self.snapshot.modes.as_mut() {
                        existing.current = modes.current;
                    } else {
                        self.snapshot.modes = Some(modes);
                    }
                } else {
                    self.snapshot.modes = Some(modes);
                }
                ThreadChange::Modes
            }
            ThreadDelta::ConfigOptions(options) => {
                self.snapshot.config_options = options;
                ThreadChange::ConfigOptions
            }
            ThreadDelta::SessionInfo(_) => ThreadChange::Metadata,
            ThreadDelta::Unknown(_) => ThreadChange::None,
        };
        self.snapshot.revision = self.snapshot.revision.saturating_add(1);
        change
    }

    fn apply_message_chunk(
        &mut self,
        id: String,
        protocol_id: Option<String>,
        role: MessageRole,
        text: String,
    ) -> ThreadChange {
        let index = self.snapshot.items.iter().position(|item| {
            matches!(item,
            ThreadItem::Message(message) if message.id == id && message.role == role)
        });
        match self.replayed_chunk(&id, role, &text) {
            ReplayChunk::Skip => ThreadChange::None,
            ReplayChunk::Replace(accumulated) => {
                if let Some(ThreadItem::Message(message)) =
                    index.map(|index| &mut self.snapshot.items[index])
                {
                    message.text = accumulated;
                }
                index.map_or(ThreadChange::None, |index| ThreadChange::Item {
                    index,
                    appended: false,
                })
            }
            ReplayChunk::Append => {
                if let Some(ThreadItem::Message(message)) =
                    index.map(|index| &mut self.snapshot.items[index])
                {
                    message.text.push_str(&text);
                    ThreadChange::Item {
                        index: index.unwrap(),
                        appended: false,
                    }
                } else {
                    self.snapshot.items.push(ThreadItem::Message(Message {
                        id,
                        protocol_id,
                        role,
                        text,
                    }));
                    ThreadChange::Item {
                        index: self.snapshot.items.len() - 1,
                        appended: true,
                    }
                }
            }
        }
    }

    fn item_index(&self, id: &str) -> Option<usize> {
        self.snapshot.items.iter().position(|item| match item {
            ThreadItem::Message(message) => message.id == id,
            ThreadItem::Content(content) => content.id == id,
            ThreadItem::Thought(thought) => thought.id == id,
            ThreadItem::Tool(tool) => tool.id == id,
        })
    }

    fn consume_local_user_echo(&mut self, id: &str, text: &str) -> LocalUserEcho {
        let Some(ThreadItem::Message(local)) = self.snapshot.items.last() else {
            return LocalUserEcho::NoMatch;
        };
        if local.role != MessageRole::User
            || !local.id.starts_with("local-message-")
            || local.protocol_id.is_some()
        {
            return LocalUserEcho::NoMatch;
        }
        let local_text = local.text.clone();
        let progress = self.user_echo_progress.entry(id.to_string()).or_default();
        progress.push_str(text);
        let accumulated = progress.clone();
        if !local_text.starts_with(&accumulated) {
            if let Some(ThreadItem::Message(local)) = self.snapshot.items.last_mut() {
                local.protocol_id = Some(id.to_string());
                local.text = accumulated;
            }
            self.user_echo_progress.remove(id);
            return LocalUserEcho::Completed;
        }
        if accumulated == local_text {
            if let Some(ThreadItem::Message(local)) = self.snapshot.items.last_mut() {
                local.protocol_id = Some(id.to_string());
            }
            self.user_echo_progress.remove(id);
            LocalUserEcho::Completed
        } else {
            LocalUserEcho::Pending
        }
    }

    fn replayed_chunk(&mut self, id: &str, role: MessageRole, text: &str) -> ReplayChunk {
        let key = (id.to_string(), role);
        let Some(original) = self.restored_messages.get(&key).cloned() else {
            return ReplayChunk::Append;
        };
        let progress = self.replay_progress.entry(key.clone()).or_default();
        progress.push_str(text);
        if original.starts_with(progress.as_str()) {
            if progress == &original {
                self.restored_messages.remove(&key);
                self.replay_progress.remove(&key);
            }
            ReplayChunk::Skip
        } else {
            let accumulated = progress.clone();
            self.restored_messages.remove(&key);
            self.replay_progress.remove(&key);
            ReplayChunk::Replace(accumulated)
        }
    }

    fn message_id(&mut self, id: Option<String>, role: Option<MessageRole>) -> String {
        // Restored messages can be replayed from the beginning of the session.
        // Live chunks, however, may only extend the adjacent compatible item.
        if let Some(incoming) = id.as_deref() {
            if let Some(message) = self.snapshot.items.iter().find_map(|item| match item {
                ThreadItem::Message(message)
                    if Some(message.role) == role
                        && self
                            .restored_messages
                            .contains_key(&(message.id.clone(), message.role))
                        && protocol_id(&message.id, message.protocol_id.as_deref())
                            == Some(incoming) =>
                {
                    Some(message)
                }
                _ => None,
            }) {
                return message.id.clone();
            }
        }
        let last = self.snapshot.items.last_mut().and_then(|item| match item {
            ThreadItem::Message(message) => {
                if role == Some(message.role) {
                    Some((&message.id, &mut message.protocol_id))
                } else {
                    None
                }
            }
            ThreadItem::Thought(thought) => {
                if role.is_none() {
                    Some((&thought.id, &mut thought.protocol_id))
                } else {
                    None
                }
            }
            _ => None,
        });
        if let Some((stable_id, existing)) = last {
            if can_merge_message_chunks(protocol_id(stable_id, existing.as_deref()), id.as_deref())
            {
                if existing.is_none() {
                    *existing = id;
                }
                return stable_id.clone();
            }
        }
        if let Some(id) = id {
            if self.item_index(&id).is_none() {
                return id;
            }
        }
        loop {
            self.next_local_id = self.next_local_id.saturating_add(1);
            let kind = if role.is_none() { "thought" } else { "message" };
            let id = format!("local-{kind}-{}", self.next_local_id);
            if self.item_index(&id).is_none() {
                return id;
            }
        }
    }
}

// Snapshots written before protocol_id existed stored explicit IDs in id.
fn protocol_id<'a>(id: &'a str, explicit: Option<&'a str>) -> Option<&'a str> {
    explicit.or_else(|| {
        (!id.starts_with("local-message-") && !id.starts_with("local-thought-")).then_some(id)
    })
}

fn can_merge_message_chunks(existing: Option<&str>, incoming: Option<&str>) -> bool {
    match (existing, incoming) {
        (Some(existing), Some(incoming)) => existing == incoming,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_chunks_upsert_by_message_id() {
        let mut reducer = ThreadReducer::new();
        assert_eq!(
            reducer.apply(ThreadDelta::MessageChunk {
                id: Some("m1".into()),
                role: MessageRole::Assistant,
                text: "he".into(),
            }),
            ThreadChange::Item {
                index: 0,
                appended: true,
            }
        );
        assert_eq!(
            reducer.apply(ThreadDelta::MessageChunk {
                id: Some("m1".into()),
                role: MessageRole::Assistant,
                text: "llo".into(),
            }),
            ThreadChange::Item {
                index: 0,
                appended: false,
            }
        );
        assert_eq!(reducer.snapshot().items.len(), 1);
        assert!(
            matches!(&reducer.snapshot().items[0], ThreadItem::Message(message) if message.id == "m1" && message.text == "hello")
        );
    }

    #[test]
    fn restored_message_replay_is_deduplicated() {
        let snapshot = ThreadSnapshot {
            revision: 2,
            items: vec![ThreadItem::Message(Message {
                protocol_id: None,
                id: "m1".into(),
                role: MessageRole::Assistant,
                text: "hello".into(),
            })],
            ..Default::default()
        };
        let mut reducer = ThreadReducer::from_snapshot(snapshot);
        reducer.apply(ThreadDelta::MessageChunk {
            id: Some("m1".into()),
            role: MessageRole::Assistant,
            text: "he".into(),
        });
        reducer.apply(ThreadDelta::MessageChunk {
            id: Some("m1".into()),
            role: MessageRole::Assistant,
            text: "llo".into(),
        });

        assert!(matches!(
            &reducer.snapshot().items[0],
            ThreadItem::Message(message) if message.text == "hello"
        ));
    }

    #[test]
    fn explicit_agent_echo_reuses_the_local_user_message() {
        let mut reducer = ThreadReducer::new();
        reducer.apply(ThreadDelta::MessageChunk {
            id: None,
            role: MessageRole::User,
            text: "hello".into(),
        });
        reducer.apply(ThreadDelta::MessageChunk {
            id: Some("agent-user-1".into()),
            role: MessageRole::User,
            text: "hel".into(),
        });
        reducer.apply(ThreadDelta::MessageChunk {
            id: Some("agent-user-1".into()),
            role: MessageRole::User,
            text: "lo".into(),
        });
        assert_eq!(reducer.snapshot().items.len(), 1);
        assert!(matches!(
            &reducer.snapshot().items[0],
            ThreadItem::Message(message)
                if message.id == "local-message-1" && message.protocol_id.as_deref() == Some("agent-user-1") && message.text == "hello"
        ));
    }

    #[test]
    fn divergent_agent_echo_keeps_the_absorbed_prefix() {
        let mut reducer = ThreadReducer::new();
        reducer.apply(ThreadDelta::MessageChunk {
            id: None,
            role: MessageRole::User,
            text: "hello".into(),
        });
        reducer.apply(ThreadDelta::MessageChunk {
            id: Some("agent-user-1".into()),
            role: MessageRole::User,
            text: "hel".into(),
        });
        reducer.apply(ThreadDelta::MessageChunk {
            id: Some("agent-user-1".into()),
            role: MessageRole::User,
            text: "Xlo".into(),
        });

        assert_eq!(reducer.snapshot().items.len(), 1);
        assert!(matches!(
            reducer.snapshot().items.last(),
            Some(ThreadItem::Message(message))
                if message.id == "local-message-1" && message.protocol_id.as_deref() == Some("agent-user-1") && message.text == "helXlo"
        ));
    }

    #[test]
    fn divergent_replay_replaces_restored_message_instead_of_appending() {
        let snapshot = ThreadSnapshot {
            revision: 1,
            items: vec![ThreadItem::Message(Message {
                protocol_id: None,
                id: "m1".into(),
                role: MessageRole::Assistant,
                text: "old answer".into(),
            })],
            ..Default::default()
        };
        let mut reducer = ThreadReducer::from_snapshot(snapshot);
        reducer.apply(ThreadDelta::MessageChunk {
            id: Some("m1".into()),
            role: MessageRole::Assistant,
            text: "new answer".into(),
        });
        assert!(matches!(
            &reducer.snapshot().items[0],
            ThreadItem::Message(message) if message.text == "new answer"
        ));
    }

    #[test]
    fn multi_chunk_divergent_replay_keeps_the_absorbed_prefix() {
        let snapshot = ThreadSnapshot {
            revision: 1,
            items: vec![ThreadItem::Message(Message {
                protocol_id: None,
                id: "m1".into(),
                role: MessageRole::Assistant,
                text: "old answer".into(),
            })],
            ..Default::default()
        };
        let mut reducer = ThreadReducer::from_snapshot(snapshot);
        reducer.apply(ThreadDelta::MessageChunk {
            id: Some("m1".into()),
            role: MessageRole::Assistant,
            text: "old".into(),
        });
        reducer.apply(ThreadDelta::MessageChunk {
            id: Some("m1".into()),
            role: MessageRole::Assistant,
            text: " new".into(),
        });

        assert!(matches!(
            &reducer.snapshot().items[0],
            ThreadItem::Message(message) if message.text == "old new"
        ));
    }

    #[test]
    fn anonymous_chunks_only_merge_with_the_same_role() {
        let mut reducer = ThreadReducer::new();
        reducer.apply(ThreadDelta::MessageChunk {
            id: None,
            role: MessageRole::User,
            text: "question".into(),
        });
        reducer.apply(ThreadDelta::MessageChunk {
            id: None,
            role: MessageRole::Assistant,
            text: "answer".into(),
        });

        assert_eq!(reducer.snapshot().items.len(), 2);
    }

    #[test]
    fn duplicate_tool_updates_do_not_add_items_and_empty_fields_preserve_state() {
        let mut reducer = ThreadReducer::new();
        reducer.apply(ThreadDelta::ToolUpsert {
            id: "tool-1".into(),
            title: Some("Read".into()),
            kind: Some(ToolKind::Read),
            state: Some(ToolState::Running),
            content: None,
            locations: None,
            raw_input: None,
            raw_output: None,
            subagent_session_id: None,
        });
        reducer.apply(ThreadDelta::ToolUpsert {
            id: "tool-1".into(),
            title: Some(String::new()),
            kind: None,
            state: Some(ToolState::Completed),
            content: None,
            locations: None,
            raw_input: None,
            raw_output: None,
            subagent_session_id: None,
        });
        assert_eq!(reducer.snapshot().items.len(), 1);
        assert!(
            matches!(&reducer.snapshot().items[0], ThreadItem::Tool(tool) if tool.title == "Read" && tool.state == ToolState::Completed)
        );
    }

    #[test]
    fn current_mode_update_preserves_available_modes() {
        let mut reducer = ThreadReducer::new();
        reducer.apply(ThreadDelta::Modes(Modes {
            current: "plan".into(),
            available: vec![
                SessionMode {
                    id: "plan".into(),
                    name: "Plan".into(),
                    description: None,
                },
                SessionMode {
                    id: "build".into(),
                    name: "Build".into(),
                    description: None,
                },
            ],
        }));
        reducer.apply(ThreadDelta::Modes(Modes {
            current: "build".into(),
            available: Vec::new(),
        }));

        let modes = reducer.snapshot().modes.as_ref().unwrap();
        assert_eq!(modes.current, "build");
        assert_eq!(modes.available.len(), 2);
    }

    #[test]
    fn thought_changes_report_existing_and_appended_items() {
        let mut reducer = ThreadReducer::new();
        assert_eq!(
            reducer.apply(ThreadDelta::ThoughtChunk {
                id: Some("thought-1".into()),
                text: "thinking".into(),
            }),
            ThreadChange::Item {
                index: 0,
                appended: true,
            }
        );
        assert_eq!(
            reducer.apply(ThreadDelta::ThoughtChunk {
                id: Some("thought-1".into()),
                text: " more".into(),
            }),
            ThreadChange::Item {
                index: 0,
                appended: false,
            }
        );
        assert!(
            matches!(&reducer.snapshot().items[0], ThreadItem::Thought(thought) if thought.text == "thinking more")
        );
    }

    #[test]
    fn content_and_tool_changes_report_their_item_index() {
        let mut reducer = ThreadReducer::new();
        assert_eq!(
            reducer.apply(ThreadDelta::ContentBlock {
                id: Some("message-1".into()),
                role: MessageRole::Assistant,
                content: ToolContent::Text("one".into()),
            }),
            ThreadChange::Item {
                index: 0,
                appended: true,
            }
        );
        assert_eq!(
            reducer.apply(ThreadDelta::ToolUpsert {
                id: "tool-1".into(),
                title: Some("Read".into()),
                kind: None,
                state: Some(ToolState::Running),
                content: None,
                locations: None,
                raw_input: None,
                raw_output: None,
                subagent_session_id: None,
            }),
            ThreadChange::Item {
                index: 1,
                appended: true,
            }
        );
        assert_eq!(
            reducer.apply(ThreadDelta::ToolUpsert {
                id: "tool-1".into(),
                title: None,
                kind: None,
                state: Some(ToolState::Completed),
                content: None,
                locations: None,
                raw_input: None,
                raw_output: None,
                subagent_session_id: None,
            }),
            ThreadChange::Item {
                index: 1,
                appended: false,
            }
        );
    }

    #[test]
    fn control_changes_report_their_category() {
        let mut reducer = ThreadReducer::new();
        assert_eq!(
            reducer.apply(ThreadDelta::Plan(Plan {
                entries: Vec::new()
            })),
            ThreadChange::Plan
        );
        assert_eq!(
            reducer.apply(ThreadDelta::Usage(Usage {
                used: 1,
                size: 2,
                cost: None,
            })),
            ThreadChange::Usage
        );
        assert_eq!(
            reducer.apply(ThreadDelta::AvailableCommands(Vec::new())),
            ThreadChange::AvailableCommands
        );
        assert_eq!(
            reducer.apply(ThreadDelta::Modes(Modes {
                current: "build".into(),
                available: Vec::new(),
            })),
            ThreadChange::Modes
        );
        assert_eq!(
            reducer.apply(ThreadDelta::ConfigOptions(Vec::new())),
            ThreadChange::ConfigOptions
        );
        assert_eq!(
            reducer.apply(ThreadDelta::SessionInfo(
                serde_json::json!({"title": "new"})
            )),
            ThreadChange::Metadata
        );
        assert_eq!(
            reducer.apply(ThreadDelta::Unknown(serde_json::json!({"future": true}))),
            ThreadChange::None
        );
    }

    #[test]
    fn unknown_delta_is_safe() {
        let mut reducer = ThreadReducer::new();
        reducer.apply(ThreadDelta::Unknown(serde_json::json!({"future": true})));
        assert!(reducer.snapshot().items.is_empty());
        assert_eq!(reducer.snapshot().revision, 1);
    }
}
