//! Contract tests for the `SkillLoader` port.

use quecto::domain::skill::SkillLoader;

#[test]
fn port_is_object_safe() {
    fn _accepts_trait_object(_: &dyn SkillLoader) {}
}
