use super::*;
use crate::shell::keys::Key;

struct DefaultOnly;

impl Component for DefaultOnly {
    fn render(&mut self, width: usize) -> Vec<String> {
        vec![format!("width={width}")]
    }
}

#[test]
fn trait_default_methods_do_not_consume_input_or_mutate_rendering() {
    let mut component = DefaultOnly;

    assert_eq!(component.render(7), vec!["width=7".to_string()]);
    assert!(!component.handle_input(&Key::Enter));
    component.invalidate();
    assert_eq!(component.render(3), vec!["width=3".to_string()]);
}
