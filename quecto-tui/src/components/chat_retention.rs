use super::chat::{CHAT_RETAINED_ENTRY_CAP, Chat};

pub(super) fn trim_tail(chat: &mut Chat) {
    chat.trim_front_for_retention(chat.entry_count().saturating_sub(CHAT_RETAINED_ENTRY_CAP));
}

pub(super) fn trim_after_prefix_mutation(chat: &mut Chat, prefix_len: usize, suffix_len: usize) {
    if chat.entry_count() <= CHAT_RETAINED_ENTRY_CAP {
        return;
    }

    let suffix = chat.entries()[prefix_len..].to_vec();
    let retained = if suffix_len >= CHAT_RETAINED_ENTRY_CAP {
        let prefix_keep = prefix_len.min(CHAT_RETAINED_ENTRY_CAP / 2);
        let suffix_keep = CHAT_RETAINED_ENTRY_CAP - prefix_keep;
        let prefix_start = prefix_len.saturating_sub(prefix_keep);
        let suffix_drop = suffix_len.saturating_sub(suffix_keep);
        chat.record_retention_front_trimmed(suffix_drop);
        chat.record_retention_front_inserted(prefix_keep);
        let mut retained = chat.entries()[prefix_start..prefix_len].to_vec();
        retained.extend_from_slice(&suffix[suffix_drop..]);
        retained
    } else {
        let prefix_keep = CHAT_RETAINED_ENTRY_CAP - suffix_len;
        let prefix_start = prefix_len.saturating_sub(prefix_keep);
        chat.record_retention_front_inserted(prefix_len.saturating_sub(prefix_start));
        let mut retained = chat.entries()[prefix_start..prefix_len].to_vec();
        retained.extend(suffix);
        retained
    };

    let preserved_scroll_offset = chat.scroll_offset();
    chat.replace_entries_after_retention(retained);
    if preserved_scroll_offset > 0 {
        chat.scroll_up(preserved_scroll_offset);
    }
}
