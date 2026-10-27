//! Optional presentation grouping; never changes stored IDs or row order.
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) enum StringIdGroup {
    Entry(usize),
    Prefix { prefix: String, indices: Vec<usize> },
}

/// Repeated, nonempty first underscore parts form one level of groups.
/// Singleton prefixes and IDs without a usable prefix/suffix remain entries.
/// Indices always address the original input; labels and children sort by ID.
pub(in crate::app) fn group_string_ids<'a>(
    ids: impl IntoIterator<Item = &'a str>,
) -> Vec<StringIdGroup> {
    let ids: Vec<_> = ids.into_iter().collect();
    let prefix = |id: &'a str| {
        id.split_once('_')
            .filter(|(prefix, suffix)| !prefix.is_empty() && !suffix.is_empty())
            .map(|(prefix, _)| prefix)
    };
    let mut members: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, id) in ids.iter().enumerate() {
        if let Some(prefix) = prefix(id) {
            members.entry(prefix).or_default().push(index);
        }
    }
    let mut groups = Vec::new();
    for (prefix, indices) in &members {
        if indices.len() < 2 {
            continue;
        }
        let mut indices = indices.clone();
        indices.sort_by(|a, b| ids[*a].cmp(ids[*b]));
        groups.push((
            (*prefix).to_owned(),
            StringIdGroup::Prefix {
                prefix: (*prefix).to_owned(),
                indices,
            },
        ));
    }
    for (index, id) in ids.iter().enumerate() {
        if prefix(id)
            .and_then(|prefix| members.get(prefix))
            .is_some_and(|members| members.len() >= 2)
        {
            continue;
        }
        groups.push(((*id).to_owned(), StringIdGroup::Entry(index)));
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    groups.into_iter().map(|(_, group)| group).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_prefixes_group_without_changing_ids_or_losing_singletons() {
        let ids = ["BR_SWAP", "unusual", "BR_PICKUP", "ODD_ONE", "PLAIN"];
        assert_eq!(
            group_string_ids(ids),
            vec![
                StringIdGroup::Prefix {
                    prefix: "BR".into(),
                    indices: vec![2, 0]
                },
                StringIdGroup::Entry(3),
                StringIdGroup::Entry(4),
                StringIdGroup::Entry(1)
            ]
        );
        assert_eq!(ids[0], "BR_SWAP");
    }
    #[test]
    fn grouping_keeps_literal_case_and_ignores_empty_prefixes_or_suffixes() {
        let groups = group_string_ids(["X_", "_A", "X_MORE", "br_SWAP", "BR_PICKUP"]);
        assert!(
            groups
                .iter()
                .all(|group| matches!(group, StringIdGroup::Entry(_)))
        );
        assert_eq!(groups.len(), 5);
    }
    #[test]
    fn grouping_retains_duplicate_rows_and_every_original_index_once() {
        let groups = group_string_ids(["A_ONE", "A_ONE", "B_ONE", "B_TWO", "plain"]);
        let mut indices: Vec<_> = groups
            .into_iter()
            .flat_map(|group| match group {
                StringIdGroup::Entry(index) => vec![index],
                StringIdGroup::Prefix { indices, .. } => indices,
            })
            .collect();
        indices.sort();
        assert_eq!(indices, vec![0, 1, 2, 3, 4]);
    }
}
