//! How the snippet input server is framed and how it is restarted.
//!
//! Read off `InputServerBus` and `LinuxInputServer`
//! (`src/server/src/services/input-server/`).

use compass_core::input_server::{
    BASE_RESTART_DELAY_MS, CrashAction, LENGTH_PREFIX_BYTES, MAX_RESTART_ATTEMPTS, MessageBuffer,
    RestartPolicy, frame, restart_delay_ms,
};

#[test]
fn the_constants_are_the_cpp_ones() {
    assert_eq!(MAX_RESTART_ATTEMPTS, 5);
    assert_eq!(BASE_RESTART_DELAY_MS, 1000);
    assert_eq!(LENGTH_PREFIX_BYTES, 4);
}

#[test]
fn a_frame_is_a_little_endian_length_then_the_payload() {
    // The C++ writes the uint32_t through a reinterpret_cast, so the prefix is
    // in the machine's own order. Getting this backwards announces a
    // 16-million-byte message for a 256-byte one and then waits for ever.
    assert_eq!(frame(b"hi"), vec![2, 0, 0, 0, b'h', b'i']);
    assert_eq!(&frame(&[0u8; 256])[..4], &[0, 1, 0, 0]);
}

#[test]
fn this_framing_is_not_the_extension_workers_framing() {
    // That one is big-endian. Confusing the two is the specific mistake this
    // test exists to catch.
    let framed = frame(&[0u8; 256]);
    assert_ne!(
        &framed[..4],
        &[0, 0, 1, 0],
        "big-endian would look like this"
    );
}

#[test]
fn an_empty_payload_still_gets_a_prefix() {
    assert_eq!(frame(b""), vec![0, 0, 0, 0]);
}

#[test]
fn a_whole_message_arriving_at_once_comes_straight_back() {
    let mut buffer = MessageBuffer::new();
    assert_eq!(buffer.push(&frame(b"hello")), vec![b"hello".to_vec()]);
    assert_eq!(buffer.buffered(), 0);
}

#[test]
fn several_messages_in_one_read_all_come_back() {
    let mut buffer = MessageBuffer::new();
    let mut chunk = frame(b"one");
    chunk.extend(frame(b"two"));
    chunk.extend(frame(b"three"));

    assert_eq!(
        buffer.push(&chunk),
        vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]
    );
}

#[test]
fn a_message_split_across_reads_is_reassembled() {
    // A read can end anywhere. Handling only whole messages would drop every
    // message big enough to be split, which is every interesting one.
    let mut buffer = MessageBuffer::new();
    let framed = frame(b"hello world");

    assert!(buffer.push(&framed[..7]).is_empty(), "not complete yet");
    assert_eq!(buffer.push(&framed[7..]), vec![b"hello world".to_vec()]);
}

#[test]
fn a_prefix_split_across_reads_is_reassembled() {
    // The nastiest case: fewer than four bytes held, so the length itself
    // cannot be read yet.
    let mut buffer = MessageBuffer::new();
    let framed = frame(b"hi");

    assert!(buffer.push(&framed[..2]).is_empty());
    assert_eq!(buffer.buffered(), 2);
    assert_eq!(buffer.push(&framed[2..]), vec![b"hi".to_vec()]);
}

#[test]
fn a_read_arriving_one_byte_at_a_time_still_works() {
    let mut buffer = MessageBuffer::new();
    let framed = frame(b"drip");
    let mut received = Vec::new();
    for byte in &framed {
        received.extend(buffer.push(&[*byte]));
    }
    assert_eq!(received, vec![b"drip".to_vec()]);
}

#[test]
fn a_complete_message_followed_by_half_of_another_yields_only_the_first() {
    let mut buffer = MessageBuffer::new();
    let mut chunk = frame(b"first");
    let second = frame(b"second");
    chunk.extend_from_slice(&second[..5]);

    assert_eq!(buffer.push(&chunk), vec![b"first".to_vec()]);
    assert_eq!(buffer.buffered(), 5, "the partial second is held");
    assert_eq!(buffer.push(&second[5..]), vec![b"second".to_vec()]);
}

#[test]
fn an_empty_message_is_a_message() {
    // Zero length is a valid frame, and treating it as "nothing yet" would
    // wedge the stream on it for ever.
    let mut buffer = MessageBuffer::new();
    assert_eq!(buffer.push(&frame(b"")), vec![Vec::<u8>::new()]);
    assert_eq!(buffer.buffered(), 0);
}

#[test]
fn a_disabled_server_is_not_restarted_and_its_crash_is_not_counted() {
    // It was closed on purpose. Counting it would spend an attempt that a real
    // crash later would need.
    let mut policy = RestartPolicy::new();
    assert_eq!(policy.handle_crash(), CrashAction::Ignore);
    assert_eq!(policy.crash_count(), 0);
}

#[test]
fn the_first_crash_is_retried_after_a_second() {
    let mut policy = RestartPolicy::new();
    policy.set_enabled(true);
    assert_eq!(
        policy.handle_crash(),
        CrashAction::RestartAfter(BASE_RESTART_DELAY_MS)
    );
}

#[test]
fn each_attempt_waits_twice_as_long() {
    // Five attempts span sixteen seconds rather than five: a server failing
    // because something else is not ready yet gets time for that to change.
    let mut policy = RestartPolicy::new();
    policy.set_enabled(true);

    let delays: Vec<_> = (0..MAX_RESTART_ATTEMPTS)
        .map(|_| match policy.handle_crash() {
            CrashAction::RestartAfter(delay) => delay,
            other => panic!("expected a restart, got {other:?}"),
        })
        .collect();
    assert_eq!(delays, vec![1000, 2000, 4000, 8000, 16000]);
}

#[test]
fn the_sixth_crash_gives_up() {
    let mut policy = RestartPolicy::new();
    policy.set_enabled(true);
    for _ in 0..MAX_RESTART_ATTEMPTS {
        policy.handle_crash();
    }
    assert_eq!(policy.handle_crash(), CrashAction::GiveUp);
    assert_eq!(
        policy.handle_crash(),
        CrashAction::GiveUp,
        "and stays given up"
    );
}

#[test]
fn a_server_that_came_up_gets_its_attempts_back() {
    // One that ran for a week and then crashed should not inherit the attempts
    // it used while it was first starting.
    let mut policy = RestartPolicy::new();
    policy.set_enabled(true);
    for _ in 0..MAX_RESTART_ATTEMPTS {
        policy.handle_crash();
    }
    assert_eq!(policy.handle_crash(), CrashAction::GiveUp);

    policy.ready();
    assert_eq!(policy.crash_count(), 0);
    assert_eq!(
        policy.handle_crash(),
        CrashAction::RestartAfter(BASE_RESTART_DELAY_MS)
    );
}

#[test]
fn disabling_a_server_stops_it_being_restarted() {
    let mut policy = RestartPolicy::new();
    policy.set_enabled(true);
    policy.handle_crash();

    policy.set_enabled(false);
    assert_eq!(policy.handle_crash(), CrashAction::Ignore);
}

#[test]
fn the_delay_formula_is_the_cpp_shift() {
    assert_eq!(restart_delay_ms(1), 1000);
    assert_eq!(restart_delay_ms(2), 2000);
    assert_eq!(restart_delay_ms(5), 16_000);
}

#[test]
fn an_absurd_attempt_number_does_not_overflow() {
    // Unreachable through the policy, which gives up at five, but the function
    // is public and a shift past 63 is undefined rather than large.
    assert!(restart_delay_ms(1000) > 0);
}
