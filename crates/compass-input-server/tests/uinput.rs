//! The real `/dev/uinput`, when this machine lets the test open it.
//!
//! Creates the server's virtual keyboard and reads its own node back. It
//! never writes a key: a key written here would reach whatever the session
//! has focused. Where `/dev/uinput` is absent or not writable (containers,
//! CI, a user without the capability) the test says so and passes.

use std::path::Path;

use compass_input_server::device::{self, UinputSink};
use compass_platform_linux::keyboard as vk;

#[test]
fn the_virtual_keyboard_is_created_and_the_server_does_not_read_itself() {
    let uinput = Path::new("/dev/uinput");
    if let Err(error) = std::fs::OpenOptions::new().write(true).open(uinput) {
        eprintln!(
            "SKIPPED: {} is not writable here ({error}); the uinput integration test did not run",
            uinput.display()
        );
        return;
    }

    let mut sink = UinputSink::create().expect("the virtual keyboard is created");
    let nodes = sink.event_nodes().expect("its nodes are listed");
    let node = nodes
        .iter()
        .find(|node| device::is_event_node(node))
        .unwrap_or_else(|| panic!("an event node among {nodes:?}"));

    let Ok(file) = std::fs::File::open(node) else {
        eprintln!(
            "SKIPPED: {} was created but cannot be read without CAP_DAC_OVERRIDE",
            node.display()
        );
        return;
    };
    let read_back = evdev::Device::from_fd(file.into()).expect("an evdev node");
    assert_eq!(read_back.name(), Some(vk::device::NAME));
    assert!(
        device::is_keyboard(&read_back),
        "udev would tag it a keyboard"
    );
    assert!(
        device::open(node).expect("it opens").is_none(),
        "the server must not read what it injects"
    );
}
