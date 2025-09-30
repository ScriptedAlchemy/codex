# Async Subagent System Implementation

## Overview

This document describes the async subagent system added to Codex, which allows the main agent to spawn child conversations that run in the background without blocking the main chat flow.

## Key Features

- **Fully Async**: Subagents run independently and don't block the parent agent
- **Notification Inbox**: Parent agent receives async notifications from subagents
- **Complete Lifecycle Management**: Create, monitor, communicate with, and terminate subagents
- **Multiple Subagents**: Parent can manage multiple concurrent subagents
- **State Tracking**: Monitor subagent status (Active, Completed, Error)

## Architecture

### Core Components

#### 1. SubagentManager (`core/src/subagent.rs`)
- Manages all subagents for a parent conversation
- Tracks subagent state and lifecycle
- Maintains notification inbox for each subagent
- Thread-safe using `Arc<RwLock<>>`

#### 2. Subagent Tools (`core/src/subagent_tools.rs`)
Five tools for interacting with subagents:

- **CreateSubagent**: Spawn a new background task
- **ListSubagents**: View all active subagents and their status
- **CheckInbox**: Retrieve notifications from subagents
- **ReplyToSubagent**: Send messages to a specific subagent
- **EndSubagent**: Terminate a subagent and clean up resources

#### 3. Protocol Extensions (`protocol/src/protocol.rs`)
New operation types:
- `Op::CreateSubagent`
- `Op::ListSubagents`
- `Op::CheckInbox`
- `Op::ReplyToSubagent`
- `Op::EndSubagent`

New event types:
- `EventMsg::SubagentCreated`
- `EventMsg::SubagentsListResponse`
- `EventMsg::InboxResponse`
- `EventMsg::SubagentReplySuccess`
- `EventMsg::SubagentEnded`

#### 4. Integration (`core/src/conversation_manager.rs`)
- Each conversation now has an associated `SubagentManager`
- Managers are created automatically when conversations are spawned
- Cleaned up when conversations are removed

## Data Structures

### SubagentId
Unique identifier for each subagent (UUID-based).

### SubagentState
```rust
enum SubagentState {
    Active,
    Completed,
    Error { message: String },
}
```

### NotificationType
```rust
enum NotificationType {
    Message { content: String },
    Question { content: String },
    Completed { summary: String },
    Error { message: String },
}
```

### SubagentNotification
Contains:
- `subagent_id`: Which subagent sent this
- `timestamp`: When it was created
- `notification`: The notification content
- `read`: Whether parent has read it

### SubagentInfo
Status information about a subagent:
- `id`: Subagent identifier
- `task`: The task description
- `state`: Current state
- `created_at`: When it was created
- `last_activity`: Last update time
- `unread_count`: Number of unread notifications

## Usage Flow

### 1. Creating a Subagent
```
Parent Agent → CreateSubagent(task: "implement feature X")
           ← SubagentCreated(subagent_id: "abc-123")
```

### 2. Checking Progress
```
Parent Agent → CheckInbox(mark_as_read: true)
           ← InboxResponse(notifications: [...])
```

### 3. Communicating
```
Parent Agent → ReplyToSubagent(subagent_id: "abc-123", message: "clarification...")
           ← SubagentReplySuccess(subagent_id: "abc-123")
```

### 4. Monitoring Status
```
Parent Agent → ListSubagents()
           ← SubagentsListResponse(subagents: [
                {id: "abc-123", state: Active, unread_count: 2, ...}
              ])
```

### 5. Ending Subagent
```
Parent Agent → EndSubagent(subagent_id: "abc-123")
           ← SubagentEnded(subagent_id: "abc-123", final_state: {...})
```

## Implementation Details

### Concurrency
- All operations are async using tokio
- State protected by `RwLock` for concurrent access
- Multiple subagents can run simultaneously

### Notification System
- Each subagent maintains a queue of notifications
- Notifications are marked as read when retrieved
- Notifications sorted by timestamp (most recent first)
- Unread count tracked per subagent

### Lifecycle Management
- Subagents have their own `CodexConversation`
- Parent can end subagents at any time
- Terminal notifications (Completed/Error) update state automatically
- Cleanup happens when subagent is ended

### Integration Points
- `ConversationManager` creates a `SubagentManager` for each conversation
- Subagent manager is cleaned up when conversation is removed
- Protocol operations route to appropriate subagent manager methods

## Testing

### Unit Tests (`core/tests/subagent_test.rs`)
- Subagent creation and ID generation
- Notification handling
- State transitions
- Inbox management
- Concurrent access patterns

### Tool Tests (`core/tests/subagent_tools_test.rs`)
- Tool definition validation
- Parameter schema correctness
- Serialization/deserialization
- Unique naming
- Description quality

All tests pass successfully ✅

## Next Steps (Not Implemented Yet)

The following would be needed to make subagents fully functional:

1. **Event Loop Integration**: Wire up subagent conversations to process events
2. **Tool Call Handlers**: Implement handlers for subagent operations in the main loop
3. **Event Processing**: Auto-generate notifications when subagents emit events
4. **Error Handling**: Proper error propagation from subagent to parent
5. **Configuration**: Allow subagents to inherit or override parent config
6. **Persistence**: Consider saving subagent state in rollouts
7. **UI Support**: Add UI elements to display subagent status and notifications

## Design Decisions

### Why Async?
The core requirement was non-blocking operation. Using async/await with tokio provides:
- Natural async/await syntax
- Efficient concurrency
- Proper task scheduling
- Consistent with existing Codex architecture

### Why Notification Inbox?
Instead of callbacks or channels:
- Parent explicitly checks for updates (pull model)
- Parent controls when to process notifications
- Easier to reason about and debug
- Fits well with turn-based conversation model

### Why SubagentManager?
Centralized management provides:
- Single source of truth for subagent state
- Easy to list all subagents
- Consistent lifecycle management
- Natural place for cross-subagent operations

## API Stability

This is a new feature with the following stability considerations:
- Protocol operations are versioned
- Event types are extensible
- Core data structures use serialization for forward compatibility
- Tool definitions can be updated without breaking changes

## Performance Considerations

- `RwLock` allows concurrent reads
- Notifications stored in `VecDeque` for efficient queue operations
- Subagent lookup is O(1) via HashMap
- List operations return cloned data to avoid holding locks

## Security Considerations

- Each subagent has its own conversation with isolated state
- Parent controls subagent lifecycle
- No cross-subagent communication (only parent-subagent)
- Subagents could inherit sandbox policies from parent

## Files Modified

- `codex-rs/core/src/subagent.rs` (new)
- `codex-rs/core/src/subagent_tools.rs` (new)
- `codex-rs/core/tests/subagent_test.rs` (new)
- `codex-rs/core/tests/subagent_tools_test.rs` (new)
- `codex-rs/core/src/conversation_manager.rs` (modified)
- `codex-rs/core/src/error.rs` (modified)
- `codex-rs/core/src/lib.rs` (modified)
- `codex-rs/core/src/openai_tools.rs` (modified)
- `codex-rs/core/src/rollout/policy.rs` (modified)
- `codex-rs/protocol/src/protocol.rs` (modified)

## Summary

The async subagent system provides a solid foundation for background task execution in Codex. The architecture is clean, testable, and extensible. The remaining work involves wiring up the protocol operations to the event loop and implementing proper event processing for subagent conversations.