use crate::byte_stub::MODULES;
use crate::lookup_table::MessageModule;
use strum::IntoEnumIterator;

#[test]
fn test_module_count() {
    let messages = MessageModule::iter().count();

    assert_eq!(messages, MODULES)
}
