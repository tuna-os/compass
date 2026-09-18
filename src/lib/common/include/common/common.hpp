#pragma once
#include <array>
#include <filesystem>
#include <fstream>
#include <istream>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace vicinae {
inline constexpr std::array<std::string_view, 3> APP_SCHEMES{"vicinae", "raycast", "com.raycast"};

// true for app deeplinks such as vicinae://toggle or com.raycast:/extensions/...
bool isAppDeeplink(std::string_view url);

// Switches the CRT locale to UTF-8 on Windows so that we operate on UTF-8 rather
// than the legacy ANSI code page. No-op elsewhere.
void enableUtf8();

std::filesystem::path selfPath();
std::optional<std::filesystem::path> findHelperProgram(std::string_view program);
std::vector<std::filesystem::path> helperProgramCandidates(std::string_view program);
std::string slurp(std::istream &ifs);

std::optional<std::filesystem::path> findServerBinary();

std::filesystem::path runtimeDir();
std::filesystem::path stateDir();

/// Creates `dir` as a directory only this user can enter, or fails.
///
/// Mirrors `compass-ipc`'s `ensure_private_dir` on the Rust side (#88). A
/// directory that already exists is ADOPTED by `create_directories`, which
/// leaves its mode alone -- so a shared root like `/tmp` lets another local user
/// pre-create the path and keep write access to whatever we put inside it.
///
/// Returns false, setting `ec`, when the path exists and is a symlink, is not a
/// directory, or is any mode other than 0700. A missing path is created 0700 and
/// is ours by construction.
///
/// Checking the mode is sufficient without checking the owner: for the attack to
/// work their directory has to be writable by us, which means a permissive mode.
/// One they own at 0700 we cannot write to at all, so the bind fails safely.
///
/// On Windows this is a plain `create_directories`: the shared-/tmp problem it
/// guards against does not arise, and the POSIX mode bits do not exist.
bool ensurePrivateDir(const std::filesystem::path &dir, std::error_code &ec);
std::filesystem::path logFilePath();
std::filesystem::path serverSocketPath();

std::string currentUserName();
std::string serverSocketName();
}; // namespace vicinae
