//! Presentation rows and storage boundaries for grouped/sorted string lists.
use super::string_groups::{StringIdGroup, group_string_ids};
use blam_tags::string_list::StringList;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Sort {
    #[default]
    Index,
    StringId,
}

#[derive(Clone)]
pub(super) struct Group {
    pub prefix: String,
    /// Every stored member in ascending storage-index order, including Find-hidden members.
    pub indices: Vec<usize>,
    /// Matching members in their display order.
    pub members: Vec<usize>,
    pub open: bool,
}
impl Group {
    /// Empty is reserved for the presentation-only collection, never an ID prefix.
    pub fn is_ungrouped(&self) -> bool { self.prefix.is_empty() }
    pub fn title(&self) -> &str { if self.is_ungrouped() {"Ungrouped Strings"} else {&self.prefix} }
    pub fn first(&self) -> usize {
        *self.indices.iter().min().unwrap()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Row {
    Group(usize),
    Entry { index: usize, group: Option<usize> },
    AddGroup(usize),
    Add,
}

pub(super) struct Layout {
    pub groups: Vec<Group>,
    pub rows: Vec<Row>,
    /// Actually displayed entry rows, for shift-selection.
    pub entries: Vec<usize>,
    section_starts: Vec<usize>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn layout(
    list: &StringList,
    visible: &[usize],
    grouped: bool,
    sort: Sort,
    descending: bool,
    expanded: &BTreeSet<String>,
    filtered: bool,
    editable: bool,
) -> Layout {
    layout_with_ungrouped(list,visible,grouped,sort,descending,expanded,filtered,editable,false)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn layout_with_ungrouped(
    list: &StringList, visible: &[usize], grouped: bool, sort: Sort, descending: bool,
    expanded: &BTreeSet<String>, filtered: bool, editable: bool, collect_ungrouped: bool,
) -> Layout {
    let ids: Vec<_> = list
        .entries
        .iter()
        .map(|entry| entry.id.as_deref().unwrap_or_default())
        .collect();
    let visible: BTreeSet<_> = visible.iter().copied().collect();
    let mut groups: Vec<_> = if grouped {
        group_string_ids(ids.iter().copied())
    } else {
        (0..ids.len()).map(StringIdGroup::Entry).collect()
    };
    if grouped && collect_ungrouped {
        let indices: Vec<_> = groups.iter().filter_map(|group| match group {
            StringIdGroup::Entry(index) => Some(*index), _ => None,
        }).collect();
        groups.retain(|group| !matches!(group,StringIdGroup::Entry(_)));
        if !indices.is_empty() { groups.push(StringIdGroup::Prefix {prefix:String::new(),indices}); }
    }
    let mut sections: Vec<_> = groups
        .into_iter()
        .filter_map(|group| match group {
            StringIdGroup::Entry(index) => visible.contains(&index).then_some((
                index,
                ids[index].to_owned(),
                None,
                vec![index],
                vec![index],
            )),
            StringIdGroup::Prefix { prefix, mut indices } => {
                // The grouping helper sorts names for pickers. This editor's
                // storage-slot mapping must always use actual index order.
                indices.sort_unstable();
                let mut members: Vec<_> = indices
                    .iter()
                    .copied()
                    .filter(|index| visible.contains(index))
                    .collect();
                if members.is_empty() {
                    return None;
                }
                members.sort_by(|a, b| {
                    let order = match sort {
                        Sort::Index => a.cmp(b),
                        Sort::StringId => ids[*a].cmp(ids[*b]).then(a.cmp(b)),
                    };
                    if descending { order.reverse() } else { order }
                });
                Some((
                    *indices.iter().min().unwrap(),
                    prefix.clone(),
                    Some(prefix),
                    indices,
                    members,
                ))
            }
        })
        .collect();
    sections.sort_by(|a, b| {
        let collection = |section: &(usize,String,Option<String>,Vec<usize>,Vec<usize>)|
            section.2.as_ref().is_some_and(String::is_empty);
        let order = collection(a).cmp(&collection(b)).then_with(|| match sort {
            Sort::Index => a.0.cmp(&b.0),
            Sort::StringId => a.1.cmp(&b.1).then(a.0.cmp(&b.0)),
        });
        if descending { order.reverse() } else { order }
    });
    let mut result = Layout {
        groups: Vec::new(),
        rows: Vec::new(),
        entries: Vec::new(),
        section_starts: Vec::new(),
    };
    for (first, _, prefix, indices, members) in sections {
        if !prefix.as_ref().is_some_and(String::is_empty) { result.section_starts.push(first); }
        if let Some(prefix) = prefix {
            let open = filtered || expanded.contains(&prefix);
            let group = result.groups.len();
            result.rows.push(Row::Group(group));
            if open {
                for &index in &members {
                    result.rows.push(Row::Entry {
                        index,
                        group: Some(group),
                    });
                    result.entries.push(index);
                }
                if editable && !prefix.is_empty() {
                    result.rows.push(Row::AddGroup(group));
                }
            }
            result.groups.push(Group {
                prefix,
                indices,
                members,
                open,
            });
        } else {
            result.rows.push(Row::Entry {
                index: first,
                group: None,
            });
            result.entries.push(first);
        }
    }
    if editable {
        result.rows.push(Row::Add);
    }
    result
}

impl Layout {
    /// Keep generated additions visible beside Add String while their ID is edited.
    /// IDs are used rather than indices so unrelated storage edits cannot move a pin.
    pub fn place_new_group_entries_last(&mut self, list: &StringList, ids: &[String]) {
        if ids.is_empty() { return; }
        for group in &mut self.groups {
            group.members.sort_by_key(|index| ids.iter().position(|id| list.entries[*index].id.as_ref() == Some(id))
                .map_or(0, |position| position + 1));
        }
        let mut cursors = vec![0; self.groups.len()];
        for row in &mut self.rows {
            if let Row::Entry { index, group: Some(g) } = row {
                *index = self.groups[*g].members[cursors[*g]];
                cursors[*g] += 1;
            }
        }
        self.entries = self.rows.iter().filter_map(|row| match row {
            Row::Entry { index, .. } => Some(*index), _ => None,
        }).collect();
    }

    /// Whole groups can only be dropped at section boundaries, never inside a group.
    pub fn accepts_group_drop(&self, row: Row, below: bool) -> bool {
        if self.container_group(row).is_some_and(|group| self.groups[group].is_ungrouped()) { return false; }
        match row {
            Row::Group(g) => !self.groups[g].open || !below,
            Row::AddGroup(_) => below,
            Row::Entry { group, .. } => group.is_none(),
            Row::Add => true,
        }
    }
    /// A group gesture carries only complete prefix groups, never partial rows
    /// or the presentation-only Ungrouped Strings collection.
    pub fn group_drag_indices(&self, group: usize, selected: &BTreeSet<usize>) -> BTreeSet<usize> {
        let source=&self.groups[group];
        if !source.indices.iter().all(|index|selected.contains(index)) {
            return source.indices.iter().copied().collect();
        }
        self.groups.iter().filter(|group| !group.is_ungrouped()
            && group.indices.iter().all(|index|selected.contains(index)))
            .flat_map(|group|group.indices.iter().copied()).collect()
    }

    pub fn group_for(&self, index: usize) -> Option<usize> {
        self.groups.iter().position(|group| group.indices.contains(&index))
    }

    /// Include collapsed groups in the same order as their displayed members.
    pub fn selected_in_display_order(&self, selected: &BTreeSet<usize>) -> Vec<usize> {
        let mut indices = Vec::new();
        for row in &self.rows {
            match *row {
                Row::Group(g) => indices.extend(self.groups[g].members.iter().copied().filter(|index| selected.contains(index))),
                Row::Entry { index, group: None } if selected.contains(&index) => indices.push(index),
                _ => {}
            }
        }
        indices
    }

    pub fn accepts_row_drop(&self, indices: &BTreeSet<usize>, row: Row, _below: bool) -> bool {
        if indices.is_empty() { return false; }
        if self.row_group(row).is_some() {return true;}
        indices.iter().all(|index| self.group_for(*index).is_none_or(|group| self.groups[group].is_ungrouped()))
    }

    pub fn row_group(&self, row: Row) -> Option<usize> {
        self.container_group(row).filter(|group| !self.groups[*group].is_ungrouped())
    }

    fn container_group(&self, row: Row) -> Option<usize> {
        match row { Row::Group(group) | Row::AddGroup(group) => Some(group),
            Row::Entry {group,..} => group, Row::Add => None }
    }

    pub fn target_row(&self, index: usize) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| matches!(row, Row::Entry { index: current, .. } if *current == index))
    }
    pub fn drop_before(
        &self,
        row: Row,
        below: bool,
        descending: bool,
        whole_group: bool,
        count: usize,
    ) -> usize {
        if row == Row::Add {
            return if descending { 0 } else { count };
        }
        let group = match row {
            Row::Group(g) | Row::AddGroup(g) => Some(g),
            Row::Entry { group, .. } => group,
            Row::Add => None,
        };
        if whole_group {
            let first = group
                .map(|g| self.groups[g].first())
                .unwrap_or_else(|| match row {
                    Row::Entry { index, .. } => index,
                    _ => 0,
                });
            let section = self
                .section_starts
                .iter()
                .position(|index| *index == first)
                .unwrap();
            let gap = section + usize::from(below || matches!(row, Row::AddGroup(_)));
            return if descending {
                if gap == 0 {
                    count
                } else if gap == self.section_starts.len() {
                    0
                } else {
                    self.section_starts[gap - 1]
                }
            } else {
                self.section_starts.get(gap).copied().unwrap_or(count)
            };
        }
        match row {
            Row::Entry { index, .. } => {
                index + usize::from(if descending { !below } else { below })
            }
            Row::Group(g) => {
                if descending {
                    self.groups[g].indices.iter().max().unwrap() + 1
                } else {
                    self.groups[g].first()
                }
            }
            Row::AddGroup(g) => insertion_index(&self.groups[g], Sort::Index, descending, count),
            Row::Add => unreachable!(),
        }
    }
}

pub(super) fn insertion_index(group: &Group, sort: Sort, descending: bool, count: usize) -> usize {
    match sort {
        Sort::StringId => count,
        Sort::Index if descending => group.first(),
        Sort::Index => group.indices.iter().max().unwrap() + 1,
    }
}

pub(super) fn prefix_for(list: &StringList, index: usize) -> Option<String> {
    group_string_ids(
        list.entries
            .iter()
            .map(|entry| entry.id.as_deref().unwrap_or_default()),
    )
    .into_iter()
    .find_map(|group| match group {
        StringIdGroup::Prefix { prefix, indices } if indices.contains(&index) => Some(prefix),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_tags::string_list::{Storage, StringEntry};
    fn list(ids: &[&str]) -> StringList {
        StringList {
            storage: Storage::Multilingual,
            entries: ids
                .iter()
                .map(|id| StringEntry {
                    id: Some((*id).into()),
                    texts: vec![],
                })
                .collect(),
            languages: vec![],
            damaged_offsets: 0,
            pairs: vec![],
        }
    }
    #[test]
    fn ungrouped_collection_is_presentation_only_and_pinned_to_the_sort_edge() {
        let source=list(&["solo_pickup","A_two","plain","A_one","B_one","B_two"]);
        for sort in [Sort::Index,Sort::StringId] {
            for descending in [false,true] {
                let shown=layout_with_ungrouped(&source,&[0,1,2,3,4,5],true,sort,descending,
                    &BTreeSet::from([String::new()]),false,true,true);
                let collection=shown.groups.iter().position(Group::is_ungrouped).unwrap();
                assert_eq!(collection,if descending {0}else{shown.groups.len()-1});
                assert_eq!(shown.groups[collection].indices,vec![0,2]);
                assert_eq!(shown.groups[collection].title(),"Ungrouped Strings");
                assert!(shown.groups[collection].open);
                assert!(!shown.rows.contains(&Row::AddGroup(collection)));
                assert_eq!(shown.row_group(Row::Group(collection)),None,"collection has no destination prefix");
                assert!(!shown.accepts_group_drop(Row::Group(collection),false));
                assert!(!shown.accepts_row_drop(&BTreeSet::from([1]),Row::Group(collection),false));
                let a=shown.group_for(1).unwrap();
                assert!(shown.accepts_row_drop(&BTreeSet::from([0,2]),Row::Group(a),true));
                let filtered=layout_with_ungrouped(&source,&[0],true,sort,descending,&BTreeSet::new(),true,true,true);
                assert_eq!(filtered.groups[0].members,vec![0]);
                assert_eq!(filtered.groups[0].indices,vec![0,2]);
                assert!(filtered.groups[0].open,"Find reveals matching collection members");
            }
        }
        let regular=list(&["A_one","A_two"]);
        assert!(!layout_with_ungrouped(&regular,&[0,1],true,Sort::Index,false,&BTreeSet::new(),false,true,true)
            .groups.iter().any(Group::is_ungrouped));
    }

    #[test]
    fn row_drags_can_enter_groups_but_keep_grouped_members_off_top_level() {
        for descending in [false, true] {
            let list = list(&["A_one", "B_one", "A_two", "B_two", "plain"]);
            let layout = layout(&list, &[0,1,2,3,4], true, Sort::Index, descending,
                &BTreeSet::from(["A".into(), "B".into()]), false, true);
            let a = layout.group_for(0).unwrap();
            let b = layout.group_for(1).unwrap();
            let selected = BTreeSet::from([0,1,2,4]);
            assert_eq!(layout.selected_in_display_order(&selected), if descending {vec![4,1,2,0]} else {vec![0,2,1,4]});
            for below in [false, true] {
                assert!(layout.accepts_row_drop(&BTreeSet::from([0]), Row::Entry {index:2, group:Some(a)}, below));
                assert!(layout.accepts_row_drop(&BTreeSet::from([0]), Row::Entry {index:1, group:Some(b)}, below));
                assert!(!layout.accepts_row_drop(&BTreeSet::from([0]), Row::Entry {index:4, group:None}, below));
                assert!(layout.accepts_row_drop(&BTreeSet::from([0,1]), Row::Entry {index:2, group:Some(a)}, below));
                assert!(layout.accepts_row_drop(&BTreeSet::from([4]), Row::Entry {index:2, group:Some(a)}, below));
            }
            assert!(layout.accepts_row_drop(&BTreeSet::from([0]), Row::Group(a), true));
            assert!(layout.accepts_row_drop(&BTreeSet::from([0]), Row::Group(a), false));
            assert!(layout.accepts_row_drop(&BTreeSet::from([0]), Row::AddGroup(a), false));
            assert!(layout.accepts_row_drop(&BTreeSet::from([0]), Row::AddGroup(a), true));
            assert!(!layout.accepts_row_drop(&BTreeSet::from([0]), Row::Add, true));
        }
    }

    #[test]
    fn groups_follow_first_stored_index_without_mutating_interleaved_members() {
        let list = list(&["B_TWO", "A_ONE", "B_ONE", "plain", "A_TWO"]);
        let layout = layout(
            &list,
            &[0, 1, 2, 3, 4],
            true,
            Sort::Index,
            false,
            &BTreeSet::new(),
            false,
            true,
        );
        assert_eq!(
            layout
                .groups
                .iter()
                .map(|g| g.prefix.as_str())
                .collect::<Vec<_>>(),
            vec!["B", "A"]
        );
        assert_eq!(layout.groups[0].members, vec![0, 2]);
        assert_eq!(
            layout.rows,
            vec![
                Row::Group(0),
                Row::Group(1),
                Row::Entry {
                    index: 3,
                    group: None
                },
                Row::Add
            ]
        );
        assert_eq!(list.entries[0].id.as_deref(), Some("B_TWO"));
    }
    #[test]
    fn filtering_keeps_total_members_but_only_reveals_matching_rows() {
        let list = list(&["A_ONE", "B_ONE", "A_TWO", "B_TWO"]);
        let layout = layout(
            &list,
            &[2],
            true,
            Sort::Index,
            false,
            &BTreeSet::new(),
            true,
            false,
        );
        assert_eq!(layout.groups[0].indices.len(), 2);
        assert_eq!(layout.groups[0].members, vec![2]);
        assert_eq!(layout.entries, vec![2]);
        assert!(layout.groups[0].open);
    }
    #[test]
    fn descending_and_alphabetical_display_keep_storage_indices() {
        let list = list(&["B_TWO", "A_ONE", "B_ONE", "A_TWO"]);
        let expanded = BTreeSet::from(["A".into(), "B".into()]);
        let descending = layout(
            &list,
            &[0, 1, 2, 3],
            true,
            Sort::Index,
            true,
            &expanded,
            false,
            false,
        );
        assert_eq!(descending.entries, vec![3, 1, 2, 0]);
        let alpha = layout(
            &list,
            &[0, 1, 2, 3],
            true,
            Sort::StringId,
            false,
            &expanded,
            false,
            false,
        );
        assert_eq!(alpha.entries, vec![1, 3, 2, 0]);
    }
    #[test]
    fn whole_group_drop_indicators_skip_every_groups_interior() {
        let list = list(&["A_one", "A_two", "plain"]);
        let layout = layout(&list, &[0, 1, 2], true, Sort::Index, false,
            &BTreeSet::from(["A".into()]), false, true);
        assert!(layout.accepts_group_drop(Row::Group(0), false));
        assert!(!layout.accepts_group_drop(Row::Group(0), true));
        for below in [false, true] {
            assert!(!layout.accepts_group_drop(Row::Entry { index: 0, group: Some(0) }, below));
            assert!(layout.accepts_group_drop(Row::Entry { index: 2, group: None }, below));
        }
        assert!(!layout.accepts_group_drop(Row::AddGroup(0), false));
        assert!(layout.accepts_group_drop(Row::AddGroup(0), true));
        let collapsed = super::layout(&list, &[0, 1, 2], true, Sort::Index, false,
            &BTreeSet::new(), false, true);
        assert!(collapsed.accepts_group_drop(Row::Group(0), false));
        assert!(collapsed.accepts_group_drop(Row::Group(0), true));
        assert!(collapsed.accepts_group_drop(Row::Add, false));
    }

    #[test]
    fn additions_and_group_drops_map_both_index_directions_to_storage_boundaries() {
        let list = list(&["A_ONE", "B_ONE", "A_TWO", "plain", "B_TWO"]);
        for descending in [false, true] {
            let layout = layout(
                &list,
                &[0, 1, 2, 3, 4],
                true,
                Sort::Index,
                descending,
                &BTreeSet::new(),
                false,
                false,
            );
            let a = layout.groups.iter().position(|g| g.prefix == "A").unwrap();
            assert_eq!(
                insertion_index(&layout.groups[a], Sort::Index, descending, 5),
                if descending { 0 } else { 3 }
            );
            assert_eq!(
                insertion_index(&layout.groups[a], Sort::StringId, descending, 5),
                5
            );
            let before = layout.drop_before(layout.rows[0], false, descending, true, 5);
            let after = layout.drop_before(*layout.rows.last().unwrap(), true, descending, true, 5);
            assert_eq!((before, after), if descending { (5, 0) } else { (0, 5) });
        }
    }
}
