#pragma once
#include "generated/tsapi.hpp"

// The Qt engine does not broker host programs for extensions: that is the
// Rust engine's HostCommand broker (crates/vicinae/src/host_commands.rs), which
// asks the person first. Here every call is refused by name.
class ExtHostCommandService : public tsapi::AbstractHostCommand {
public:
  explicit ExtHostCommandService(tsapi::RpcTransport &transport) : AbstractHostCommand(transport) {}

  tsapi::Result<tsapi::HostCommandResult>::Future run(tsapi::HostCommandRequest request) override {
    return tsapi::Result<tsapi::HostCommandResult>::fail(
        "HostCommand/run is not supported by this engine: it cannot run " + request.program + " on the host");
  }
};
