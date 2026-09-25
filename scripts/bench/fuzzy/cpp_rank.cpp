// Upstream Vicinae's fuzzy scorer, timed the way `fuzzy-throughput` times
// Compass's: score every haystack line, keep accepted matches, stable-sort by
// score, take the top 20. Single-threaded, as upstream ranks.
//
// Build (no Qt, no CMake; vicinae::fuzzy is header-only):
//   c++ -std=c++23 -O2 -Iscripts/bench/upstream/fuzzy/include -o cpp-rank scripts/bench/fuzzy/cpp_rank.cpp
// Run:
//   cpp-rank HAYSTACK ITERATIONS QUERY...
// Prints one tab-separated line per query: query, matches, median_ns, p95_ns.
#include <algorithm>
#include <chrono>
#include <cstddef>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>

#include "fuzzy/fuzzy-searchable.hpp"

namespace {

struct Hit {
  std::size_t index;
  int score;
};

constexpr std::size_t TOP_N = 20;

std::size_t rank(const std::vector<std::string> &items, const fuzzy::Query &query) {
  std::vector<Hit> hits;
  hits.reserve(items.size());
  for (std::size_t i = 0; i < items.size(); ++i) {
    auto const m = fuzzy::scoreWeighted({{items[i], 1.0}}, query);
    if (m.accepted()) hits.push_back({i, m.score});
  }
  std::stable_sort(hits.begin(), hits.end(), [](Hit const &a, Hit const &b) { return a.score > b.score; });
  auto const matched = hits.size();
  hits.resize(std::min(hits.size(), TOP_N));
  volatile std::size_t sink = hits.empty() ? 0 : hits.front().index;
  (void)sink;
  return matched;
}

} // namespace

int main(int argc, char **argv) {
  if (argc < 4) {
    std::cerr << "usage: cpp-rank HAYSTACK ITERATIONS QUERY...\n";
    return 2;
  }
  std::vector<std::string> items;
  std::ifstream file(argv[1]);
  for (std::string line; std::getline(file, line);) {
    if (!line.empty()) items.push_back(line);
  }
  auto const iterations = static_cast<std::size_t>(std::stoul(argv[2]));
  for (int q = 3; q < argc; ++q) {
    fuzzy::Query const query{std::string{argv[q]}};
    auto const matched = rank(items, query);
    for (int warm = 0; warm < 10; ++warm) rank(items, query);
    std::vector<long long> samples;
    samples.reserve(iterations);
    for (std::size_t i = 0; i < iterations; ++i) {
      auto const start = std::chrono::steady_clock::now();
      rank(items, query);
      auto const end = std::chrono::steady_clock::now();
      samples.push_back(std::chrono::duration_cast<std::chrono::nanoseconds>(end - start).count());
    }
    std::sort(samples.begin(), samples.end());
    std::cout << argv[q] << '\t' << matched << '\t' << samples[samples.size() / 2] << '\t'
              << samples[(samples.size() * 95) / 100] << '\n';
  }
  return 0;
}
