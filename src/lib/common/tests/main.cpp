// Tests for `ensurePrivateDir`, the guard on directories that land in a shared
// root.
//
// These touch the real filesystem on purpose. What is being tested is how the
// kernel answers `lstat` and `mkdir` for modes, symlinks and non-directories,
// and a fake would only re-state the assumption under test.
//
// CONTROLLED: replacing the body of `ensurePrivateDir` with the
// `fs::create_directories` it replaced fails three of these -- the 0700 mode,
// the refusal of a 0777 directory, and the refusal of a symlink. The middle one
// is the vulnerability in #59 and #88 stated as a test.

#include <catch2/catch_test_macros.hpp>

#include "common/common.hpp"

#include <filesystem>
#include <fstream>
#include <sys/stat.h>
#include <unistd.h>

namespace fs = std::filesystem;

namespace {
/// A directory under the system temp dir, removed when the test ends.
class Scratch {
public:
  Scratch() : m_root(fs::temp_directory_path() / ("vicinae-common-tests-" + std::to_string(::getpid()))) {
    fs::remove_all(m_root);
    fs::create_directories(m_root);
  }
  ~Scratch() { fs::remove_all(m_root); }
  fs::path operator/(std::string_view leaf) const { return m_root / leaf; }

private:
  fs::path m_root;
};

/// The permission bits of `p` itself, not of what it points at.
unsigned mode_of(const fs::path &p) {
  return static_cast<unsigned>(fs::symlink_status(p).permissions()) & 07777;
}
} // namespace

TEST_CASE("ensurePrivateDir creates a missing directory as 0700") {
  Scratch scratch;
  const auto dir = scratch / "fresh";
  std::error_code ec;

  REQUIRE(vicinae::ensurePrivateDir(dir, ec));
  REQUIRE_FALSE(ec);
  REQUIRE(fs::is_directory(dir));

  // The mode is the point. `create_directories` would apply the umask, giving
  // 0755 on a typical system and leaving the directory readable by everyone.
  REQUIRE(mode_of(dir) == 0700);
}

TEST_CASE("ensurePrivateDir accepts a directory that is already 0700") {
  Scratch scratch;
  const auto dir = scratch / "ours";
  std::error_code ec;

  REQUIRE(vicinae::ensurePrivateDir(dir, ec));
  REQUIRE(vicinae::ensurePrivateDir(dir, ec));
  REQUIRE_FALSE(ec);
}

TEST_CASE("ensurePrivateDir refuses a directory another user could write to") {
  // THE VULNERABILITY, AS A TEST. An attacker pre-creates the path with a
  // permissive mode; `create_directories` succeeds on it and leaves the mode
  // alone, so the socket we then bind inside it is theirs to unlink and
  // replace.
  Scratch scratch;
  const auto dir = scratch / "worldwritable";
  REQUIRE(::mkdir(dir.c_str(), 0777) == 0);
  REQUIRE(::chmod(dir.c_str(), 0777) == 0);

  std::error_code ec;
  REQUIRE_FALSE(vicinae::ensurePrivateDir(dir, ec));
  REQUIRE(ec);
}

TEST_CASE("ensurePrivateDir refuses a symlink rather than following it") {
  // Not covered by the mode check above: a symlink can point anywhere we can
  // write, so its own mode says nothing useful. Hence lstat rather than stat.
  Scratch scratch;
  const auto target = scratch / "target";
  const auto link = scratch / "link";
  fs::create_directory(target);
  REQUIRE(::symlink(target.c_str(), link.c_str()) == 0);

  std::error_code ec;
  REQUIRE_FALSE(vicinae::ensurePrivateDir(link, ec));
  REQUIRE(ec);
}

TEST_CASE("ensurePrivateDir refuses a path that is not a directory") {
  Scratch scratch;
  const auto file = scratch / "afile";
  { std::ofstream(file) << "x"; }

  std::error_code ec;
  REQUIRE_FALSE(vicinae::ensurePrivateDir(file, ec));
  REQUIRE(ec);
}

TEST_CASE("the shared-root fallbacks are per-user") {
  // A flat /tmp/vicinae is one directory for every user on the machine. The uid
  // does not make the path unguessable -- ensurePrivateDir is what protects it
  // -- it stops users colliding with each other in the ordinary case.
  const auto runtime = vicinae::runtimeDir();
  const auto state = vicinae::stateDir();
  const auto uid = std::to_string(::getuid());

  for (const auto &dir : {runtime, state}) {
    if (dir.string().starts_with("/tmp/")) { REQUIRE(dir.filename().string() == "vicinae-" + uid); }
  }
}
