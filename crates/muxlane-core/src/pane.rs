//! PaneTree：递归 split + 每 pane TabGroup，纯模型可单测。
use crate::model::{new_id, AgentId};
use serde::{Deserialize, Serialize};

pub type PaneId = String;
fn new_split_id() -> String {
    new_id("split")
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SplitAxis {
    Horizontal, // 左右
    Vertical,   // 上下
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TabGroup {
    pub id: PaneId,
    #[serde(default)]
    pub tabs: Vec<AgentId>,
    pub active: Option<AgentId>,
}

impl TabGroup {
    pub fn new() -> Self {
        Self {
            id: new_id("pane"),
            tabs: vec![],
            active: None,
        }
    }
    pub fn with_tab(agent: AgentId) -> Self {
        Self {
            id: new_id("pane"),
            tabs: vec![agent.clone()],
            active: Some(agent),
        }
    }
    pub fn open(&mut self, agent: AgentId) {
        if !self.tabs.contains(&agent) {
            self.tabs.push(agent.clone());
        }
        self.active = Some(agent);
    }
    pub fn close(&mut self, agent: &AgentId) {
        let old_ix = self.tabs.iter().position(|a| a == agent);
        self.tabs.retain(|a| a != agent);
        if self.active.as_ref() == Some(agent) {
            self.active = if self.tabs.is_empty() {
                None
            } else {
                let ix = old_ix.unwrap_or(0).min(self.tabs.len().saturating_sub(1));
                Some(self.tabs[ix].clone())
            };
        }
    }
    pub fn reorder(&mut self, from: usize, to: usize) -> bool {
        if from >= self.tabs.len() || to >= self.tabs.len() || from == to {
            return false;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        true
    }
}

impl Default for TabGroup {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaneNode {
    Leaf {
        group: TabGroup,
    },
    Split {
        #[serde(default = "new_split_id")]
        id: String,
        axis: SplitAxis,
        children: Vec<PaneNode>,
        /// 比例，与 children 等长；归一化到 1.0
        sizes: Vec<f32>,
    },
}

impl PaneNode {
    pub fn empty() -> Self {
        PaneNode::Leaf {
            group: TabGroup::new(),
        }
    }
    pub fn with_tab(agent: AgentId) -> Self {
        PaneNode::Leaf {
            group: TabGroup::with_tab(agent),
        }
    }

    pub fn first_pane_id(&self) -> PaneId {
        match self {
            PaneNode::Leaf { group } => group.id.clone(),
            PaneNode::Split { children, .. } => children
                .first()
                .map(|c| c.first_pane_id())
                .unwrap_or_default(),
        }
    }

    pub fn group(&self, id: &PaneId) -> Option<&TabGroup> {
        match self {
            PaneNode::Leaf { group } if &group.id == id => Some(group),
            PaneNode::Leaf { .. } => None,
            PaneNode::Split { children, .. } => children.iter().find_map(|c| c.group(id)),
        }
    }
    pub fn group_mut(&mut self, id: &PaneId) -> Option<&mut TabGroup> {
        match self {
            PaneNode::Leaf { group } if &group.id == id => Some(group),
            PaneNode::Leaf { .. } => None,
            PaneNode::Split { children, .. } => children.iter_mut().find_map(|c| c.group_mut(id)),
        }
    }

    pub fn pane_for_agent(&self, agent: &AgentId) -> Option<PaneId> {
        match self {
            PaneNode::Leaf { group } if group.tabs.contains(agent) => Some(group.id.clone()),
            PaneNode::Leaf { .. } => None,
            PaneNode::Split { children, .. } => {
                children.iter().find_map(|c| c.pane_for_agent(agent))
            }
        }
    }

    pub fn open_tab(&mut self, pane: &PaneId, agent: AgentId) -> bool {
        if let Some(g) = self.group_mut(pane) {
            g.open(agent);
            true
        } else {
            false
        }
    }

    /// 显式分屏：目标 leaf 变成 Split[原 leaf, 新 leaf(agent)]。
    pub fn split(&mut self, pane: &PaneId, axis: SplitAxis, new_agent: AgentId) -> Option<PaneId> {
        match self {
            PaneNode::Leaf { group } if &group.id == pane => {
                let old = group.clone();
                let new_group = TabGroup::with_tab(new_agent);
                let new_id = new_group.id.clone();
                *self = PaneNode::Split {
                    id: new_split_id(),
                    axis,
                    children: vec![
                        PaneNode::Leaf { group: old },
                        PaneNode::Leaf { group: new_group },
                    ],
                    sizes: vec![0.5, 0.5],
                };
                Some(new_id)
            }
            PaneNode::Split { children, .. } => {
                for c in children {
                    if let Some(id) = c.split(pane, axis, new_agent.clone()) {
                        return Some(id);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// 移动 tab：同 pane 重排或跨 pane 移动。`target_index` 是插入槽（0..=len）。
    pub fn move_tab(
        &mut self,
        from: &PaneId,
        to: &PaneId,
        agent: &AgentId,
        target_index: usize,
    ) -> bool {
        if from == to {
            let Some(g) = self.group_mut(from) else {
                return false;
            };
            let Some(old) = g.tabs.iter().position(|a| a == agent) else {
                return false;
            };
            let tab = g.tabs.remove(old);
            let adjusted = if old < target_index {
                target_index.saturating_sub(1)
            } else {
                target_index
            };
            let ix = adjusted.min(g.tabs.len());
            g.tabs.insert(ix, tab.clone());
            g.active = Some(tab);
            return true;
        }
        let exists = self.group(from).is_some_and(|g| g.tabs.contains(agent));
        if !exists {
            return false;
        }
        if let Some(g) = self.group_mut(from) {
            g.close(agent);
        }
        if let Some(g) = self.group_mut(to) {
            let ix = target_index.min(g.tabs.len());
            g.tabs.insert(ix, agent.clone());
            g.active = Some(agent.clone());
            true
        } else {
            // 回滚到 source
            if let Some(g) = self.group_mut(from) {
                g.open(agent.clone());
            }
            false
        }
    }
    pub fn close_tab(&mut self, pane: &PaneId, agent: &AgentId) -> bool {
        if let Some(g) = self.group_mut(pane) {
            if !g.tabs.contains(agent) {
                return false;
            }
            g.close(agent);
            true
        } else {
            false
        }
    }

    /// 清理已不存在的 agent tab（重启恢复布局骨架时使用）。
    pub fn retain_agents(&mut self, valid: &std::collections::HashSet<AgentId>) {
        match self {
            PaneNode::Leaf { group } => {
                group.tabs.retain(|a| valid.contains(a));
                if group.active.as_ref().is_some_and(|a| !valid.contains(a)) {
                    group.active = group.tabs.last().cloned();
                }
            }
            PaneNode::Split { children, .. } => {
                for child in children {
                    child.retain_agents(valid);
                }
            }
        }
    }

    pub fn split_info(&self, split_id: &str) -> Option<(SplitAxis, Vec<f32>)> {
        match self {
            PaneNode::Split {
                id,
                axis,
                sizes,
                children,
            } => {
                if id == split_id {
                    Some((*axis, sizes.clone()))
                } else {
                    children.iter().find_map(|c| c.split_info(split_id))
                }
            }
            PaneNode::Leaf { .. } => None,
        }
    }

    pub fn update_split_sizes(&mut self, split_id: &str, mut next: Vec<f32>) -> bool {
        match self {
            PaneNode::Split {
                id,
                sizes,
                children,
                ..
            } => {
                if id == split_id && next.len() == children.len() {
                    if next.iter().any(|value| !value.is_finite() || *value <= 0.0) {
                        return false;
                    }
                    let total: f32 = next.iter().sum();
                    for v in &mut next {
                        *v = (*v / total).max(0.05);
                    }
                    let total: f32 = next.iter().sum();
                    if total > 0.0 {
                        for v in &mut next {
                            *v /= total;
                        }
                    }
                    *sizes = next;
                    true
                } else {
                    children
                        .iter_mut()
                        .any(|c| c.update_split_sizes(split_id, next.clone()))
                }
            }
            PaneNode::Leaf { .. } => false,
        }
    }

    /// 删除 pane，并递归折叠只剩一个 child 的 split。
    pub fn without_pane(&self, pane: &PaneId) -> Option<PaneNode> {
        match self {
            PaneNode::Leaf { group } => (group.id != *pane).then(|| self.clone()),
            PaneNode::Split {
                id,
                axis,
                children,
                sizes,
            } => {
                let mut kept = Vec::new();
                let mut kept_sizes = Vec::new();
                for (ix, child) in children.iter().enumerate() {
                    if let Some(next) = child.without_pane(pane) {
                        kept.push(next);
                        kept_sizes.push(*sizes.get(ix).unwrap_or(&1.0));
                    }
                }
                match kept.len() {
                    0 => None,
                    1 => kept.into_iter().next(),
                    _ => {
                        let total: f32 = kept_sizes.iter().sum();
                        if total > 0.0 {
                            for s in &mut kept_sizes {
                                *s /= total;
                            }
                        }
                        Some(PaneNode::Split {
                            id: id.clone(),
                            axis: *axis,
                            children: kept,
                            sizes: kept_sizes,
                        })
                    }
                }
            }
        }
    }
    pub fn leaf_count(&self) -> usize {
        match self {
            PaneNode::Leaf { .. } => 1,
            PaneNode::Split { children, .. } => children.iter().map(PaneNode::leaf_count).sum(),
        }
    }

    pub fn all_groups(&self) -> Vec<&TabGroup> {
        let mut out = vec![];
        self.collect_groups(&mut out);
        out
    }

    /// True when no pane holds any tab (e.g. every session is detached).
    pub fn has_no_tabs(&self) -> bool {
        self.all_groups().iter().all(|group| group.tabs.is_empty())
    }
    fn collect_groups<'a>(&'a self, out: &mut Vec<&'a TabGroup>) {
        match self {
            PaneNode::Leaf { group } => out.push(group),
            PaneNode::Split { children, .. } => {
                for c in children {
                    c.collect_groups(out);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_split_axis_is_tolerated() {
        assert_eq!(
            serde_json::from_str::<SplitAxis>(r#""diagonal""#).unwrap(),
            SplitAxis::Unknown
        );
    }
    #[test]
    fn tabs_open_close_reorder() {
        let mut g = TabGroup::with_tab("a".into());
        g.open("b".into());
        g.open("c".into());
        assert_eq!(g.active.as_deref(), Some("c"));
        assert!(g.reorder(2, 0));
        assert_eq!(g.tabs, vec!["c", "a", "b"]);
        g.close(&"c".into());
        assert_eq!(g.active.as_deref(), Some("a"));
    }

    #[test]
    fn close_tab_reports_only_real_removals() {
        let mut tree = PaneNode::with_tab("a".into());
        let pane = tree.first_pane_id();
        assert!(!tree.close_tab(&pane, &"missing".into()));
        assert!(tree.close_tab(&pane, &"a".into()));
        assert!(!tree.close_tab(&pane, &"a".into()));
    }
    #[test]
    fn activating_a_tab_preserves_its_pane_identity() {
        let mut tree = PaneNode::with_tab("a".into());
        let maximized_pane = tree.first_pane_id();

        assert!(tree.open_tab(&maximized_pane, "b".into()));
        assert_eq!(
            tree.pane_for_agent(&"b".into()),
            Some(maximized_pane.clone())
        );
        assert_eq!(
            tree.group(&maximized_pane).unwrap().active.as_deref(),
            Some("b")
        );

        let split_pane = tree
            .split(&maximized_pane, SplitAxis::Horizontal, "c".into())
            .unwrap();
        assert_ne!(split_pane, maximized_pane);
        assert!(tree.group(&maximized_pane).is_some());
    }

    #[test]
    fn explicit_split_only() {
        let mut tree = PaneNode::with_tab("a".into());
        let root = tree.first_pane_id();
        tree.open_tab(&root, "b".into()); // tab，不分屏
        assert_eq!(tree.leaf_count(), 1);
        assert_eq!(tree.group(&root).unwrap().tabs.len(), 2);
        let new_id = tree
            .split(&root, SplitAxis::Horizontal, "c".into())
            .unwrap();
        assert_eq!(tree.leaf_count(), 2);
        assert_eq!(tree.group(&new_id).unwrap().active.as_deref(), Some("c"));
    }
    #[test]
    fn move_tab_reorder_and_cross_pane() {
        let mut tree = PaneNode::with_tab("a".into());
        let p1 = tree.first_pane_id();
        tree.open_tab(&p1, "b".into());
        let p2 = tree.split(&p1, SplitAxis::Horizontal, "c".into()).unwrap();
        assert!(tree.move_tab(&p1, &p1, &"b".into(), 0));
        assert_eq!(tree.group(&p1).unwrap().tabs, vec!["b", "a"]);
        // move first to append slot len => true end
        assert!(tree.move_tab(&p1, &p1, &"b".into(), 2));
        assert_eq!(tree.group(&p1).unwrap().tabs, vec!["a", "b"]);
        assert!(tree.move_tab(&p1, &p2, &"a".into(), 0));
        assert_eq!(tree.group(&p1).unwrap().tabs, vec!["b"]);
        assert_eq!(tree.group(&p2).unwrap().tabs, vec!["a", "c"]);
    }

    #[test]
    fn remove_pane_collapses_parent_and_sizes_update() {
        let mut tree = PaneNode::with_tab("a".into());
        let p1 = tree.first_pane_id();
        let p2 = tree.split(&p1, SplitAxis::Horizontal, "b".into()).unwrap();
        let split_id = match &tree {
            PaneNode::Split { id, .. } => id.clone(),
            _ => unreachable!(),
        };
        assert!(tree.update_split_sizes(&split_id, vec![0.7, 0.3]));
        assert_eq!(tree.split_info(&split_id).unwrap().1, vec![0.7, 0.3]);
        tree = tree.without_pane(&p2).unwrap();
        assert_eq!(tree.leaf_count(), 1);
        assert_eq!(tree.group(&p1).unwrap().active.as_deref(), Some("a"));
    }

    #[test]
    fn nested_remove_collapses_each_single_child_parent() {
        let mut tree = PaneNode::with_tab("a".into());
        let pane_a = tree.first_pane_id();
        let pane_b = tree
            .split(&pane_a, SplitAxis::Horizontal, "b".into())
            .unwrap();
        let pane_c = tree
            .split(&pane_b, SplitAxis::Vertical, "c".into())
            .unwrap();
        assert_eq!(tree.leaf_count(), 3);

        tree = tree.without_pane(&pane_c).unwrap();
        assert_eq!(tree.leaf_count(), 2);
        assert!(tree.group(&pane_b).is_some());

        tree = tree.without_pane(&pane_b).unwrap();
        assert_eq!(tree.leaf_count(), 1);
        assert_eq!(tree.first_pane_id(), pane_a);
    }

    #[test]
    fn split_sizes_reject_zero_and_non_finite_values() {
        let mut tree = PaneNode::with_tab("a".into());
        let pane = tree.first_pane_id();
        tree.split(&pane, SplitAxis::Horizontal, "b".into())
            .unwrap();
        let split = match &tree {
            PaneNode::Split { id, .. } => id.clone(),
            _ => unreachable!(),
        };
        assert!(!tree.update_split_sizes(&split, vec![0.0, 0.0]));
        assert!(!tree.update_split_sizes(&split, vec![f32::NAN, 1.0]));
        assert_eq!(tree.split_info(&split).unwrap().1, vec![0.5, 0.5]);
    }

    #[test]
    fn recursive_split() {
        let mut tree = PaneNode::with_tab("a".into());
        let p1 = tree.first_pane_id();
        let p2 = tree.split(&p1, SplitAxis::Horizontal, "b".into()).unwrap();
        let _p3 = tree.split(&p2, SplitAxis::Vertical, "c".into()).unwrap();
        assert_eq!(tree.leaf_count(), 3);
        assert!(tree.pane_for_agent(&"c".into()).is_some());
    }
}
