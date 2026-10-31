//! The agent layer: a team of AI agents with their own tools, direct chats,
//! group channels, agent-to-agent messaging, approvals, memory, skills,
//! routines, connectors (MCP) and a browser. Every agent talks to whatever
//! OpenAI-compatible endpoint and model it is configured with.

pub mod browser;
pub mod engine;
pub mod llm;
pub mod mcp;
pub mod model;
pub mod persist;
pub mod prompt;
pub mod runtime;
pub mod tools;
