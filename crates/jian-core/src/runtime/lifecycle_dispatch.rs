//! Runtime lifecycle dispatch (R4 Canonical PreviewInput): spawn the
//! authored ActionLists for app / page / node lifecycle hooks
//! (`onLaunch`, `onEnter`, `onMount`, ...) through the SAME task queue
//! and ActionContext path the gesture dispatcher uses, so lifecycle
//! actions schedule exactly like event actions.
//!
//! Hook resolution is by NAME at the caller-chosen scope, and every
//! scope's `disabledEvents` list is honored: a hook listed there never
//! spawns. Unknown/extra hook keys (R1's flattened `ExtraJson`) stay
//! opaque — only the typed hook fields dispatch.
//!
//! The `$event` payload is caller-provided (normalized JSON carrying the
//! factual `reason` and the previous/next route when known); this module
//! never guesses facts the caller did not supply.

use super::Runtime;
use crate::document::tree::NodeKey;
use crate::document::RuntimeDocument;
use jian_ops_schema::lifecycle::NodeLifecycleHooks;

/// The scope a lifecycle hook resolves against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleScope {
    /// Document-level hooks (`PenDocument.lifecycle`).
    App,
    /// Page-level hooks on the page with schema id `page_id`.
    Page { page_id: String },
    /// Node-level hooks on the runtime tree node `key`.
    Node(NodeKey),
}

impl Runtime {
    /// Resolve and spawn the authored ActionList for `hook` (e.g.
    /// `"onLaunch"`, `"onEnter"`, `"onMount"`) at `scope`. `payload`
    /// becomes the ActionContext `$event` unchanged. Returns `false`
    /// when the scope has no such authored hook, the hook is listed in
    /// the scope's `disabledEvents`, the hook name is unknown to the
    /// runtime, or the task queue declined the spawn — a rejected
    /// lifecycle hook is a silent no-op, never an error surfaced to
    /// input.
    pub fn dispatch_lifecycle(
        &mut self,
        scope: &LifecycleScope,
        hook: &str,
        payload: serde_json::Value,
    ) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let Some((list, node_id)) = resolve_lifecycle_list(document, scope, hook) else {
            return false;
        };
        let mut context = self.make_action_ctx();
        context.event = Some(crate::value::RuntimeValue::from(payload));
        context.node_id = node_id;
        self.task_queue
            .spawn(
                &self.actions,
                &list,
                context,
                self.document_generation,
                Some(hook.to_owned()),
            )
            .inspect(|_| {
                // Lifecycle input is synchronous like every other input:
                // harvest the spawned hook task NOW (the deliver path
                // does the same) so `set` writes land before the caller
                // inspects state, instead of waiting for the next pump.
                self.collect_task_outcomes();
                self.scheduler.flush();
            })
            .is_ok()
    }

    /// Spawn a PRE-RESOLVED lifecycle ActionList through the same task
    /// path [`Runtime::dispatch_lifecycle`], for callers that captured
    /// the list against a document generation about to be replaced —
    /// the outgoing screen's `onLeave` / `onUnmount` during a route
    /// swap. Spawning AFTER the swap (with the new generation) keeps the
    /// swap's `cancel_all_except` from killing them, while the resolved
    /// list + node id carry the outgoing scope across. The caller is
    /// responsible for `disabledEvents` (it resolved the list itself).
    pub fn spawn_lifecycle(
        &mut self,
        hook: &str,
        list: serde_json::Value,
        node_id: Option<String>,
        payload: serde_json::Value,
    ) -> bool {
        let mut context = self.make_action_ctx();
        context.event = Some(crate::value::RuntimeValue::from(payload));
        context.node_id = node_id;
        self.task_queue
            .spawn(
                &self.actions,
                &list,
                context,
                self.document_generation,
                Some(hook.to_owned()),
            )
            .is_ok()
    }

    /// Collect the runtime tree keys whose authored `lifecycle` block
    /// declares `hook` (e.g. `"onUnmount"`), in tree order. Hosts call
    /// this BEFORE a document swap so unmount hooks dispatch against the
    /// outgoing tree, then dispatch each key through
    /// [`Runtime::dispatch_lifecycle`].
    pub fn nodes_with_lifecycle_hook(&self, hook: &str) -> Vec<NodeKey> {
        let Some(document) = self.document.as_ref() else {
            return Vec::new();
        };
        document
            .tree
            .nodes
            .iter()
            .filter(|(_, node)| {
                // A hook listed in the node's own `disabledEvents` never
                // fires — the same rule `dispatch_lifecycle` enforces.
                node.schema.lifecycle().is_some_and(|hooks| {
                    !hook_is_disabled(hooks.disabled_events.as_deref(), hook)
                        && node_declares_hook(hooks, hook)
                })
            })
            .map(|(key, _)| key)
            .collect()
    }
}

/// The serialized ActionList + target node schema id for `hook` at
/// `scope`, or `None` when the scope has no authored (and enabled) hook
/// by that name.
fn resolve_lifecycle_list(
    document: &RuntimeDocument,
    scope: &LifecycleScope,
    hook: &str,
) -> Option<(serde_json::Value, Option<String>)> {
    let schema = &document.schema;
    let tree = &document.tree;
    match scope {
        LifecycleScope::App => {
            let list = schema
                .lifecycle
                .as_ref()
                .filter(|hooks| !hook_is_disabled(hooks.disabled_events.as_deref(), hook))
                .and_then(|hooks| match hook {
                    "onLaunch" => hooks.on_launch.as_ref(),
                    "onResume" => hooks.on_resume.as_ref(),
                    "onBackground" => hooks.on_background.as_ref(),
                    "onTerminate" => hooks.on_terminate.as_ref(),
                    _ => None,
                })?;
            Some((serde_json::to_value(list).ok()?, None))
        }
        LifecycleScope::Page { page_id } => {
            let list = schema
                .pages
                .as_ref()
                .and_then(|pages| pages.iter().find(|page| &page.id == page_id))
                .and_then(|page| page.lifecycle.as_ref())
                .filter(|hooks| !hook_is_disabled(hooks.disabled_events.as_deref(), hook))
                .and_then(|hooks| match hook {
                    "onEnter" => hooks.on_enter.as_ref(),
                    "onLeave" => hooks.on_leave.as_ref(),
                    "onForeground" => hooks.on_foreground.as_ref(),
                    "onBackground" => hooks.on_background.as_ref(),
                    _ => None,
                })?;
            Some((serde_json::to_value(list).ok()?, None))
        }
        LifecycleScope::Node(key) => {
            let node = tree.nodes.get(*key)?;
            let hooks = node.schema.lifecycle()?;
            if hook_is_disabled(hooks.disabled_events.as_deref(), hook) {
                return None;
            }
            let list = hooks.lifecycle_list(hook)?;
            let schema_id = crate::document::tree::node_schema_id(&node.schema).to_owned();
            Some((serde_json::to_value(list).ok()?, Some(schema_id)))
        }
    }
}

/// The node-scope ActionList for `hook` (mount/unmount only — other hook
/// names are unknown to the node scope), or `None` when not authored.
trait NodeLifecycleList {
    fn lifecycle_list(&self, hook: &str) -> Option<&jian_ops_schema::events::ActionList>;
}

impl NodeLifecycleList for NodeLifecycleHooks {
    fn lifecycle_list(&self, hook: &str) -> Option<&jian_ops_schema::events::ActionList> {
        match hook {
            "onMount" => self.on_mount.as_ref(),
            "onUnmount" => self.on_unmount.as_ref(),
            _ => None,
        }
    }
}

/// Whether the node's hooks struct authors `hook` (mount/unmount only —
/// other hook names are unknown to the node scope).
fn node_declares_hook(hooks: &NodeLifecycleHooks, hook: &str) -> bool {
    hooks.lifecycle_list(hook).is_some()
}

/// `true` when the scope's `disabledEvents` list contains `hook`.
fn hook_is_disabled(disabled: Option<&[String]>, hook: &str) -> bool {
    disabled.is_some_and(|list| list.iter().any(|name| name == hook))
}
