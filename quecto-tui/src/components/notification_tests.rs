use super::*;

fn strip_ansi(s: &str) -> String {
    let mut result = String::new();
    let mut in_escape = false;
    for ch in s.chars() {
        if in_escape {
            if ch.is_ascii_alphabetic() || ch == '~' {
                in_escape = false;
            }
        } else if ch == '\x1b' {
            in_escape = true;
        } else {
            result.push(ch);
        }
    }
    result
}

#[test]
fn renders_message() {
    let mut n = Notification::new("Saved!", NotifyLevel::Success);
    let lines = n.render(40);
    assert_eq!(lines.len(), 1);
    let plain = strip_ansi(&lines[0]);
    assert!(plain.contains("Saved!"), "should show message: {}", plain);
}

#[test]
fn expired_renders_empty() {
    let mut n =
        Notification::new("test", NotifyLevel::Info).with_duration(Duration::from_millis(0));
    std::thread::sleep(Duration::from_millis(1));
    let lines = n.render(40);
    assert!(lines.is_empty());
}

#[test]
fn info_level_icon() {
    let mut n = Notification::new("info", NotifyLevel::Info);
    let lines = n.render(40);
    let plain = strip_ansi(&lines[0]);
    assert!(plain.contains("ℹ"), "should have info icon: {}", plain);
}

#[test]
fn error_level_icon() {
    let mut n = Notification::new("error", NotifyLevel::Error);
    let lines = n.render(40);
    let plain = strip_ansi(&lines[0]);
    assert!(plain.contains("✗"), "should have error icon: {}", plain);
}

#[test]
fn stack_gc_removes_expired() {
    let mut stack = NotificationStack::new();
    stack.push(Notification::new("old", NotifyLevel::Info).with_duration(Duration::from_millis(0)));
    std::thread::sleep(Duration::from_millis(1));
    assert!(stack.gc());
    assert!(stack.is_empty());
}

#[test]
fn stack_limits_size() {
    let mut stack = NotificationStack::new();
    for i in 0..10 {
        stack.push(Notification::new(&format!("msg{}", i), NotifyLevel::Info));
    }
    assert!(stack.notifications.len() <= MAX_NOTIFICATIONS);
}
