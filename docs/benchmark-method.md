# Benchmark method and limits

Final selected-build results are pending. The retained benchmark code supports correctness-gated warm-cache comparisons using explicit roots, binary hashes, tool commands, per-round timing, CPU usage and peak resident memory. Compare summary-only tools separately from tools retaining a complete browsable tree. Different accounting and output contracts require explicit handling.

`bench/harness.py` supplies owned synthetic fixtures and an independent lstat accounting oracle. `bench/paired.py` and `paired_sparse.py` serialize observations; `paired_stats.py` estimates paired median ratios and bootstrap intervals. `bench/verify_binary.py` checks a supplied binary on disposable fixtures. `bench/ratatui_readiness.py` observes completed terminal frames without retaining path-bearing transcripts. These scripts do not establish that a published result was executed; the final result receipt must bind the commands, binary and source.

Use a fresh owned directory for generated fixtures. Do not benchmark while compiling or running another scanner. Report unfavorable cases and correctness exclusions. Warm local Intel Mac/APFS measurements do not cover cold caches, HDDs, network/FUSE filesystems, Apple Silicon or other hardware. The project has no persistent index and makes no claim of universal superiority or a physical optimum.
