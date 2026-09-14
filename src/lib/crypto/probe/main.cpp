// Suite 0: expose the C++ clipboard crypto so it can be diffed against the Rust
// port. See docs/rust-engine/PLAN.md §8.4a.
//
// WHY THIS IS AFFORDABLE
//
// `vicinae::crypto` is a standalone static library whose only dependency on
// Linux is OpenSSL — no Qt, no CMake needed to consume it. Three translation
// units compile in about a second, which is the same property that made the
// fuzzy scorer probe (src/lib/fuzzy/probe) cheap enough to run on every PR.
// Phase 3's gate asks that the clipboard store be "readable and writable by
// both engines interchangeably"; this is what lets that be checked per-PR
// rather than only in the VM tier.
//
// WHY A BYTE-DIFF IS THE WRONG TEST, AND WHAT THIS SUPPORTS INSTEAD
//
// The scorer is a pure function, so the two engines' outputs can be compared
// directly. `encrypt` is not: the IV comes from `RAND_bytes`, so two CORRECT
// implementations produce different bytes on every call. A harness that
// compared ciphertexts would fail on a correct port, which is the same mistake
// as a ratchet that fires when a number improves.
//
// So this speaks a small request/response protocol instead, and the driver uses
// it for CROSS-DECRYPTION: C++ encrypts and Rust decrypts, then the reverse.
// That is what "interchangeably" means, and it is stronger than a byte-diff --
// it proves each side can consume what the other produces, rather than that
// both happen to agree on one code path.
//
// `deriveKey` IS deterministic (HKDF-SHA256), so that one diffs directly.
//
// WHY ERRORS ARE NAMED RATHER THAN COLLAPSED
//
// `decrypt` returns a typed `DecryptError`, and the driver's tamper control
// asserts the specific value `AuthFailed`. A probe that printed a bare "err"
// would let a decrypt that ignored the GCM tag pass the control, because
// "it errored" is true of almost any bug. The names are the point.
//
// PROTOCOL: one request per line on stdin, one response per line on stdout.
// All byte strings are lowercase hex, including the KDF label, so that no
// quoting or separator question can arise.
//
//   derive  <master-hex> <label-hex>       -> ok <subkey-hex> | err <Name>
//   encrypt <key-hex> <plaintext-hex>      -> ok <blob-hex>   | err <Name>
//   decrypt <key-hex> <blob-hex>           -> ok <plain-hex>  | err <Name>
//   genkey                                 -> ok <key-hex>    | err <Name>
//
// An EMPTY byte string is written `-`, never as an empty field. Whitespace
// separated fields cannot express "present and empty", so without a sentinel an
// empty KDF label or an empty plaintext -- both legitimate, and both in the
// driver's corpus -- would arrive as a missing argument and be rejected as a
// malformed request. `-` is not a hex digit, so it cannot collide with data.
//
// An unparseable line is `err BadRequest`, never a silent skip: the driver
// counts responses against requests, so a probe that quietly dropped a line
// would otherwise look like agreement.
#include <iostream>
#include <optional>
#include <span>
#include <sstream>
#include <string>
#include <vector>

#include "crypto/aes-gcm.hpp"
#include "crypto/kdf.hpp"

namespace {

std::optional<std::vector<std::byte>> unhex(std::string_view text) {
  if (text == "-") return std::vector<std::byte>{};
  if (text.empty()) return std::nullopt;
  if (text.size() % 2 != 0) return std::nullopt;
  std::vector<std::byte> out;
  out.reserve(text.size() / 2);
  for (std::size_t i = 0; i < text.size(); i += 2) {
    int value = 0;
    for (int nibble = 0; nibble < 2; ++nibble) {
      const char c = text[i + nibble];
      int digit;
      if (c >= '0' && c <= '9') {
        digit = c - '0';
      } else if (c >= 'a' && c <= 'f') {
        digit = c - 'a' + 10;
      } else if (c >= 'A' && c <= 'F') {
        digit = c - 'A' + 10;
      } else {
        return std::nullopt;
      }
      value = value * 16 + digit;
    }
    out.push_back(static_cast<std::byte>(value));
  }
  return out;
}

std::string hex(std::span<const std::byte> bytes) {
  static constexpr char DIGITS[] = "0123456789abcdef";
  if (bytes.empty()) return "-";
  std::string out;
  out.reserve(bytes.size() * 2);
  for (std::byte b : bytes) {
    const auto value = static_cast<unsigned char>(b);
    out.push_back(DIGITS[value >> 4]);
    out.push_back(DIGITS[value & 0x0f]);
  }
  return out;
}

const char *name(Crypto::AES256GCM::DecryptError error) {
  using E = Crypto::AES256GCM::DecryptError;
  switch (error) {
  case E::InvalidKeySize:
    return "InvalidKeySize";
  case E::DataTooShort:
    return "DataTooShort";
  case E::CipherError:
    return "CipherError";
  case E::AuthFailed:
    return "AuthFailed";
  }
  return "UnknownDecryptError";
}

} // namespace

int main() {
  std::ios::sync_with_stdio(false);

  std::string line;
  while (std::getline(std::cin, line)) {
    std::istringstream fields(line);
    std::string op;
    fields >> op;

    if (op == "genkey") {
      auto key = Crypto::AES256GCM::generateKey();
      if (!key) {
        std::cout << "err NoRandomness\n";
        continue;
      }
      std::cout << "ok " << hex(*key) << "\n";
      continue;
    }

    std::string first, second;
    if (!(fields >> first >> second)) {
      std::cout << "err BadRequest\n";
      continue;
    }

    const auto a = unhex(first);
    const auto b = unhex(second);
    if (!a || !b) {
      std::cout << "err BadHex\n";
      continue;
    }

    if (op == "derive") {
      // The label is hex on the wire and text to the library, so a label with a
      // space or a non-ASCII byte cannot be mangled by this protocol.
      const std::string label(reinterpret_cast<const char *>(b->data()), b->size());
      auto subkey = Crypto::deriveKey(*a, label);
      if (!subkey) {
        std::cout << "err DeriveFailed\n";
        continue;
      }
      std::cout << "ok " << hex(*subkey) << "\n";
    } else if (op == "encrypt") {
      auto blob = Crypto::AES256GCM::encrypt(*b, *a);
      if (!blob) {
        std::cout << "err CipherError\n";
        continue;
      }
      std::cout << "ok " << hex(*blob) << "\n";
    } else if (op == "decrypt") {
      auto plain = Crypto::AES256GCM::decrypt(*b, *a);
      if (!plain) {
        std::cout << "err " << name(plain.error()) << "\n";
        continue;
      }
      std::cout << "ok " << hex(*plain) << "\n";
    } else {
      std::cout << "err BadRequest\n";
    }
  }

  return 0;
}
