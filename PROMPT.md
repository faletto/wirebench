# Task: produce a detailed engineering plan for an open-source network simulator

You are acting as a staff-level engineer and technical program planner. I want a
detailed, opinionated, buildable plan, not a survey of options. Where you have to
choose, choose, and tell me why in one or two sentences. Flag the decisions
where you think I'm wrong.

## Project summary

I'm building a clean-room, open-source replacement for Cisco Packet Tracer, aimed
at CCNA-level students, self-learners, and instructors. Working name: `wirebench`.

Motivation: Packet Tracer is slow (startup, UI responsiveness, and simulation
under load), closed source, gated behind Netacad login, and installs poorly on
school hardware. The goal is a fast, scriptable, git-friendly simulator that
people can run from a URL or as a native app.

## Decisions already made (don't relitigate unless you see a serious flaw)

1. **Language and targets:** Rust. A pure-Rust core library with no browser or OS
   dependencies. Build targets: native (desktop, CLI, tests) and
   WebAssembly (browser). Web-first for distribution, but native must stay
   first-class, because responsiveness is the main goal.
2. **UI:** egui/eframe, compiled natively and to WASM from one codebase.
   Challenge this only if you see a concrete problem with large topologies,
   terminal emulation, text rendering, or accessibility. If you do, propose the
   alternative.
3. **Simulation engine:** a native, in-process, deterministic discrete-event
   simulator running on **virtual time** (pausable, steppable, fast-forwardable,
   reproducible). No containers, no real OS images, no FRR/Containerlab at
   runtime. All protocols are implemented in Rust.
4. **IOS-style CLI:** the CLI fidelity is the product. It should be a
   data-driven command tree (modes, `?` help, tab completion, abbreviations,
   IOS-like error messages and `show` output formats), with each protocol
   module contributing its own commands.
5. **Cisco file import (`.pkt` / `.pka`):** deliberately limited. The importer
   accepts only **already-decoded XML**. The project will not contain code,
   keys, or algorithms for decrypting or decoding Cisco's file containers,
   will not disassemble or debug Cisco software, and will not write `.pkt`
   files. How a user obtains XML is out of scope. `.pka` activity/grading logic is
   out of scope for the first releases.
6. **Legal and relationship posture:** observation-only. Behavior is derived from
   public standards (IEEE/RFC), public Cisco documentation, behavior observed
   through legitimate use, and our own testing. The project must stay on good
   terms with Cisco, since I may work there someday. No copying of Cisco assets,
   icons, IOS help text, or output strings wholesale. Treat this as an
   explicit design constraint, not an afterthought.

## Scope

**In scope (CCNA blueprint, phased by you):**
- L2: Ethernet, MAC learning, VLANs, 802.1Q trunking, STP/RSTP, EtherChannel (LACP/PAgP-equivalent), port security
- L3: ARP, ICMP, static routing, OSPFv2 (single-area first, then multi-area), possibly EIGRP (decide and justify), NAT/PAT, ACLs, DHCP server/relay, basic IPv6
- Services/hosts: DNS, HTTP, TFTP/FTP-style basics, hosts/PCs with a simple CLI and a `ping`/`traceroute`/`ipconfig` toolset
- Management plane: console/VTY lines, `enable` secrets, `show running-config`, `copy run start`, hostnames/banners
- A simulation-mode **packet inspector** (step through frames, per-hop PDU view) as a first-class feature, since Packet Tracer's version of this is a major teaching tool
- A text-based, versioned lab file format (YAML or RON) designed for git diffs

**Non-goals (confirm or push back):** wireless, IoT devices, VoIP, full BGP, hardware-accurate timing, real IOS image support, Cisco account integration, and `.pkt` export.

## Constraints and quality bars

- Performance targets (propose concrete numbers and a benchmarking plan): cold start to interactive, UI frame time with ~100 nodes, simulation event throughput, and OSPF/STP convergence speed under fast-forward, on both native and WASM.
- Determinism: same topology plus same inputs must give byte-identical traces.
- The core must be fuzzable (packet parsers) and property-testable.
- I'm one developer, with occasional contributors later. Plan for that. Prefer fewer, sharper components and boring technology.

## What I want back

Produce a single structured document with these sections:

1. **Architecture overview:** a crate/workspace layout (names, responsibilities, dependency directions), with a diagram in text form. Include the sim kernel (virtual clock, event queue, link model), the packet/types layer, the device model (host/switch/router built from protocol modules), the protocol-module trait/interface, a shared RIB/forwarding layer, the config model, the CLI engine, persistence, and the UI layer. Show how the WASM and native builds differ, and where a Web Worker boundary belongs.
2. **Core interface sketches:** Rust trait/struct signatures (not full implementations) for the protocol-module interface, the event/clock API, the CLI command-tree schema, and the lab-file schema. Explain the trade-offs you made.
3. **CLI engine design:** how to represent modes, command grammar, `?` help, completion, abbreviation rules, and `show` output templating so that it's data-driven. Explain how to get IOS-like feel without copying Cisco text, and how to test fidelity.
4. **Protocol roadmap:** an ordered list of protocols/features with, for each: the relevant RFC/standard, the core behaviors to implement, the known pitfalls, a test strategy, and a rough size estimate (S/M/L/XL). Justify the order.
5. **Milestones:** a phased plan (M0 spike, M1 vertical slice, M2 MVP, M3 CCNA-complete, beyond), each with a concrete demo/exit criterion, an estimated effort range for one developer, and the main risks. M1 should be the smallest end-to-end slice that proves the architecture (suggest: two hosts, a switch, and a router with static routes and `ping`, with the IOS shell in the egui/WASM app).
6. **UI/UX plan:** the topology canvas (rendering approach and how to keep it fast at scale), a terminal widget in egui, the packet inspector, device config panels, accessibility, and keyboard-first workflows.
7. **Testing and verification strategy:** golden-scenario tests, protocol conformance testing, fuzzing, property tests (such as STP never forwards in a loop), CI setup for native and WASM, performance regression benchmarks. Mention how to use real implementations as *test oracles* (for example comparing against captured behavior) without making them runtime dependencies, and flag any licensing issues.
8. **Import design (XML only):** how to structure an importer that reads decoded XML into the lab model, with a plugin or adapter layer, an "unsupported features" report, unknown-device fallbacks, and version tolerance. Do not describe how to decode Cisco's container format.
9. **Legal/provenance guardrails:** a short, practical contributor policy (what sources are allowed, what isn't), a provenance log format, naming/branding cautions (no confusing similarity with Cisco marks), licensing recommendations for the code (and why), and what to run by an IP lawyer before public launch. Note that you are not a lawyer.
10. **Risks and open questions:** the top ten, ranked, each with a mitigation or a decision I need to make. Include things you think I'm underestimating.
11. **First two weeks:** a day-by-day or task-by-task plan I can start on immediately, with the repo scaffold, CI, and the first failing tests.

## Style

- Be concrete: crate names, type names, file layouts, RFC numbers.
- Use tables where comparison helps and prose where reasoning matters.
- Mark each claim you're unsure about as `[verify]` so I can check it.
- Lead each section with the recommendation, then the justification.
- If something I've specified is contradictory or risky, say so before planning around it.
