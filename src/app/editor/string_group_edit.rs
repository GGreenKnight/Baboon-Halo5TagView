//! Bulk prefix operations; display groups never change stored indices on rename.
use blam_tags::string_list::{StringEntry, StringList, StringListEdit};
use std::collections::BTreeSet;

pub(super) struct RenamePlan {
    pub target: String,
    pub merge: bool,
    pub indices: Vec<usize>,
    pub edits: Vec<StringListEdit>,
}

pub(super) fn normalize_prefix(text: &str) -> Result<String, String> {
    let prefix = text.trim_end_matches('_');
    if prefix.is_empty() || prefix.chars().any(|c| c == '_' || c.is_whitespace()) {
        return Err("Enter a prefix without spaces or internal underscores. Trailing underscores are ignored.".into());
    }
    Ok(prefix.into())
}

fn member<'a>(id: &'a str, prefix: &str) -> Option<&'a str> {
    let (owner, suffix) = id.split_once('_')?;
    (owner == prefix && !suffix.is_empty()).then_some(suffix)
}

fn prefixes(list: &StringList) -> BTreeSet<&str> {
    list.entries
        .iter()
        .filter_map(|entry| {
            let (prefix, suffix) = entry.id.as_deref()?.split_once('_')?;
            (!prefix.is_empty() && !suffix.is_empty()).then_some(prefix)
        })
        .collect()
}

pub(super) fn rename(list: &StringList, prefix: &str, text: &str) -> Result<RenamePlan, String> {
    let target = normalize_prefix(text)?;
    let members: Vec<_> = list
        .entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            member(entry.id.as_deref()?, prefix).map(|suffix| (index, format!("{target}_{suffix}")))
        })
        .collect();
    if members.len() < 2 {
        return Err("This group no longer exists. Open its menu again.".into());
    }
    let indices: Vec<_> = members.iter().map(|(index, _)| *index).collect();
    let merge = target != prefix && prefixes(list).contains(target.as_str());
    if target == prefix {
        return Ok(RenamePlan {
            target,
            merge,
            indices,
            edits: vec![],
        });
    }
    let mut taken: BTreeSet<_> = list
        .entries
        .iter()
        .enumerate()
        .filter(|(index, _)| !indices.contains(index))
        .filter_map(|(_, entry)| entry.id.clone())
        .collect();
    // Reserve natural incoming IDs before allocating numbered collisions.
    let reserved: BTreeSet<_> = taken
        .iter()
        .cloned()
        .chain(members.iter().map(|(_, id)| id.clone()))
        .collect();
    let edits = members
        .into_iter()
        .map(|(index, base)| {
            let id = if taken.insert(base.clone()) {
                base
            } else {
                let id = (1..)
                    .map(|number| format!("{base}({number})"))
                    .find(|id| !reserved.contains(id) && !taken.contains(id))
                    .expect("unbounded");
                taken.insert(id.clone());
                id
            };
            StringListEdit::SetId { index, id }
        })
        .collect();
    Ok(RenamePlan {
        target,
        merge,
        indices,
        edits,
    })
}

pub(super) struct TransferPlan {
    pub at: usize,
    pub count: usize,
    pub edits: Vec<StringListEdit>,
}

/// Transfer a row range into a prefix, preserving natural IDs whenever free.
pub(super) fn transfer(
    list: &StringList, ordered: &[usize], before: usize, prefix: &str, copy: bool,
) -> Result<TransferPlan, String> {
    let indices: BTreeSet<_> = ordered.iter().copied().collect();
    if indices.is_empty() || indices.len() != ordered.len() || indices.iter().any(|&index| index >= list.entries.len()) {
        return Err("The dragged strings changed. Start the drag again.".into());
    }
    let mut entries = Vec::new();
    for &index in ordered {
        let mut entry = list.entries[index].clone();
        let id = entry.id.as_deref().unwrap_or("new_string");
        let suffix = id.split_once('_').filter(|(owner,suffix)| !owner.is_empty() && !suffix.is_empty())
            .map_or(id, |(_,suffix)| suffix);
        entry.id = Some(format!("{prefix}_{suffix}"));
        entries.push(entry);
    }
    let mut taken: BTreeSet<_> = list.entries.iter().enumerate()
        .filter(|(index,_)| copy || !indices.contains(index)).filter_map(|(_,entry)|entry.id.clone()).collect();
    // Existing destination members keep their IDs when moved, even when an
    // earlier incoming member would otherwise claim the same natural name.
    let keep: Vec<_> = ordered.iter().map(|&index| !copy && list.entries[index].id.as_deref()
        .is_some_and(|id| member(id,prefix).is_some())).collect();
    taken.extend(entries.iter().zip(&keep).filter(|(_,keep)| **keep).filter_map(|(entry,_)|entry.id.clone()));
    let reserved: BTreeSet<_> = taken.iter().cloned().chain(entries.iter().filter_map(|entry|entry.id.clone())).collect();
    for (entry,keep) in entries.iter_mut().zip(keep) {
        if keep {continue;}
        let base = entry.id.as_ref().unwrap();
        if !taken.insert(base.clone()) {
            let id = (1..).map(|n|format!("{base}({n})")).find(|id| !taken.contains(id) && !reserved.contains(id)).expect("unbounded");
            taken.insert(id.clone());
            entry.id = Some(id);
        }
    }
    let before = before.min(list.entries.len());
    let at = if copy { before } else { before-indices.iter().filter(|&&index|index<before).count() };
    let count = entries.len();
    let mut edits = Vec::new();
    if !copy { edits.push(StringListEdit::Remove {indices:indices.iter().copied().collect()}); }
    edits.push(StringListEdit::Insert {at,entries});
    Ok(TransferPlan {at,count,edits})
}

pub(super) fn duplicate(
    list: &StringList,
    prefix: &str,
) -> Result<(String, Vec<StringEntry>), String> {
    duplicate_many(list,&[prefix]).map(|mut copies|copies.remove(0))
}

/// Reserve each generated prefix before planning the next copy, so related
/// prefixes such as pr and pr(1) cannot receive the same destination name.
pub(super) fn duplicate_many(list: &StringList, sources: &[&str]) -> Result<Vec<(String,Vec<StringEntry>)>,String> {
    let mut used: BTreeSet<String>=prefixes(list).into_iter().map(str::to_owned).collect();
    sources.iter().map(|prefix|duplicate_reserved(list,prefix,&mut used)).collect()
}

fn duplicate_reserved(list: &StringList, prefix: &str, used: &mut BTreeSet<String>) -> Result<(String,Vec<StringEntry>),String> {
    let base = prefix
        .strip_suffix(')')
        .and_then(|stem| stem.rsplit_once('('))
        .filter(|(base, number)| !base.is_empty() && number.parse::<usize>().is_ok())
        .map_or(prefix, |(base, _)| base);
    let target = (1..)
        .map(|number| format!("{base}({number})"))
        .find(|prefix| !used.contains(prefix.as_str()))
        .expect("unbounded");
    let mut taken = BTreeSet::new();
    let entries: Vec<_> = list
        .entries
        .iter()
        .filter_map(|entry| {
            let suffix = member(entry.id.as_deref()?, prefix)?;
            let mut entry = entry.clone();
            let base = format!("{target}_{suffix}");
            let id = if taken.insert(base.clone()) {
                base
            } else {
                let id = (1..)
                    .map(|number| format!("{base}({number})"))
                    .find(|id| !taken.contains(id))
                    .expect("unbounded");
                taken.insert(id.clone());
                id
            };
            entry.id = Some(id);
            Some(entry)
        })
        .collect();
    if entries.len() < 2 {
        return Err("This group no longer exists. Open its menu again.".into());
    }
    used.insert(target.clone());
    Ok((target, entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn list(ids: &[&str]) -> StringList {
        StringList {
            storage: blam_tags::string_list::Storage::Multilingual,
            entries: ids
                .iter()
                .enumerate()
                .map(|(index, id)| StringEntry {
                    id: Some((*id).into()),
                    texts: vec![
                        Some(format!("English {index}")),
                        Some(format!("Japanese {index}")),
                    ],
                })
                .collect(),
            languages: vec![],
            damaged_offsets: 0,
            pairs: vec![],
        }
    }
    #[test]
    fn cross_group_copy_keeps_natural_names_and_all_languages() {
        let source=list(&["old_pickup","old_swap","new_other","new_hint"]);
        let plan=transfer(&source,&[0,1],3,"new",true).unwrap();
        assert_eq!(plan.at,3);
        assert_eq!(plan.count,2);
        let [StringListEdit::Insert {at:3,entries}]=&plan.edits[..] else {panic!("unexpected transfer")};
        assert_eq!(entries[0].id.as_deref(),Some("new_pickup"));
        assert_eq!(entries[1].id.as_deref(),Some("new_swap"));
        assert_eq!(entries[0].texts,source.entries[0].texts);
        assert_eq!(entries[1].texts,source.entries[1].texts);
    }

    #[test]
    fn cross_group_move_numbers_only_collisions_and_preserves_natural_incoming_ids() {
        let source=list(&["old_one","old_one(1)","new_one","new_keep"]);
        let plan=transfer(&source,&[0,1],4,"new",false).unwrap();
        assert_eq!(plan.at,2);
        let [StringListEdit::Remove {indices},StringListEdit::Insert {at:2,entries}]=&plan.edits[..] else {panic!("unexpected transfer")};
        assert_eq!(indices,&vec![0,1]);
        assert_eq!(entries[0].id.as_deref(),Some("new_one(2)"));
        assert_eq!(entries[1].id.as_deref(),Some("new_one(1)"));
        assert_eq!(entries[0].texts,source.entries[0].texts);
    }

    #[test]
    fn prefix_rename_preserves_suffixes_indices_and_ignores_trailing_underscores() {
        let list = list(&["pr_pickup", "other", "pr_Swap_ai"]);
        let plan = rename(&list, "pr", "plasma___").unwrap();
        assert!(!plan.merge);
        assert_eq!(plan.indices, vec![0, 2]);
        assert!(
            matches!(&plan.edits[..], [StringListEdit::SetId {index:0,id:a},StringListEdit::SetId {index:2,id:b}]
            if a=="plasma_pickup" && b=="plasma_Swap_ai")
        );
        assert_eq!(list.entries[0].id.as_deref(), Some("pr_pickup"));
        for invalid in ["", "___", "pr_new", " pr", "pr ", "two words"] {
            assert!(normalize_prefix(invalid).is_err());
        }
        assert!(rename(&list, "pr", "pr_").unwrap().edits.is_empty());
    }
    #[test]
    fn merge_numbers_only_incoming_collisions_and_reserves_natural_names() {
        let list = list(&[
            "old_pickup",
            "new_pickup",
            "old_pickup(1)",
            "old_swap",
            "new_swap",
            "new_swap(1)",
        ]);
        let plan = rename(&list, "old", "new").unwrap();
        assert!(plan.merge);
        assert!(
            matches!(&plan.edits[..], [StringListEdit::SetId {index:0,id:a},StringListEdit::SetId {index:2,id:b},StringListEdit::SetId {index:3,id:c}]
            if a=="new_pickup(2)" && b=="new_pickup(1)" && c=="new_swap(2)")
        );
    }
    #[test]
    fn copying_related_groups_reserves_distinct_generated_prefixes() {
        let source=list(&["pr_pickup","pr(1)_pickup","pr_swap","pr(1)_swap"]);
        let copies=duplicate_many(&source,&["pr","pr(1)"]).unwrap();
        assert_eq!(copies.iter().map(|(prefix,_)|prefix.as_str()).collect::<Vec<_>>(),vec!["pr(2)","pr(3)"]);
        assert_eq!(copies[0].1[0].texts,source.entries[0].texts);
        assert_eq!(copies[1].1[0].texts,source.entries[1].texts);
    }

    #[test]
    fn duplicated_group_gets_a_unique_prefix_and_every_translation() {
        let list = list(&["pr_pickup", "pr(1)_pickup", "pr_swap"]);
        let (prefix, entries) = duplicate(&list, "pr").unwrap();
        assert_eq!(prefix, "pr(2)");
        assert_eq!(entries[0].id.as_deref(), Some("pr(2)_pickup"));
        assert_eq!(entries[1].id.as_deref(), Some("pr(2)_swap"));
        assert_eq!(entries[0].texts, list.entries[0].texts);
        assert_eq!(entries[1].texts, list.entries[2].texts);
    }
}
