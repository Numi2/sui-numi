NUMI SUI STACK
 Low‑Latency Aggregation and Execution on Sui

⸻

Numi Sui Stack is a architecture under dev for building an ultra‑low‑latency trade and data aggregator on Sui. It combines: (1) Sui’s object‑centric fast‑path execution model to avoid consensus when safe; (2) DeepBook V3 as the native, fully on‑chain CLOB; (3) Sui’s modern data plane via gRPC and the GraphQL RPC + General‑Purpose Indexer; and (4) an execution path that exploits Programmable Transaction Blocks (PTBs), sponsored transactions, and the new Transaction Driver introduced alongside Mysticeti v2. The result is a competitive stack designed for sub‑second confirmations, high parallelism, and clean operational boundaries between routing, data, execution, and controls. Sui’s deprecation of JSON‑RPC by April 2026 and the availability of typed gRPC and GraphQL APIs anchors the roadmap and informs all interface choices.  ￼

⸻

1. Motivation and Market Context

Aggregation on a modern L1 must solve for three conflicting forces: tail‑latency under bursty load, on‑chain atomicity across heterogeneous venues, and data richness with verifiable freshness. Purely off‑chain routing is fast but brittle; purely on‑chain routing is verifiable but can be slow. Sui’s hybrid architecture, which distinguishes owned from shared objects and supports a fast path for purely owned‑object transactions, lets Numi Sui Stack combine both worlds: keep hot paths on owned objects and touch consensus only when market structure demands global ordering.  ￼

DeepBook V3 makes this practical by providing a native CLOB with order options, a shared BalanceManager abstraction, and an official indexer for precise market state reads. This reduces adapter complexity and allows the router to price across pools and order books without bespoke ETL.  ￼

⸻

2. Sui Foundations Relevant to Our Design

Object‑centric execution and parallelism. Sui models state as objects. Transactions that touch different owned objects can execute in parallel without global ordering, while transactions that mutate shared objects are ordered by consensus. This is the core lever Numi uses to minimize contention and reduce tail latency.  ￼

Fast path vs consensus path. If a transaction involves only owned objects, Sui executes it immediately after certification—no consensus wait—whereas shared‑object transactions enter the consensus engine. Numi routes to owned‑object venues or owned‑state call patterns whenever price is comparable, and prices in the extra latency cost when shared objects are unavoidable.  ￼

Mysticeti v2 and the Transaction Driver. Sui’s consensus has evolved from Narwhal/Bullshark to Mysticeti, a low‑latency uncertified‑DAG protocol with a 3‑round commit path in the steady state. With Mysticeti v2, Sui integrates a Transaction Driver that replaces the Quorum Driver, simplifying submission and reducing observed confirmation latency; Sui plans it as default from node v1.60. Numi colocates with performant validators and uses this driver to minimize the number of network hops and signatures on the hot path.  ￼

Modern data interfaces. Sui’s gRPC full node API is type‑safe and low‑latency, and the GraphQL RPC + General‑Purpose Indexer (Beta) provides a structured view over checkpoint‑processed state with archival connectivity. JSON‑RPC is officially deprecated with a stated migration timeline of April 2026, which Numi adheres to from day one.  ￼

Programmable Transaction Blocks (PTBs). PTBs let us compose up to 1,024 commands atomically in one transaction, which is ideal for multi‑venue routes, flash‑loan‑backed arbs, or cancel/replace chains. The router’s route plans compile to PTBs with predictable gas and deterministic failure modes.  ￼

⸻

3. DeepBook V3 Integration

DeepBook V3 is Sui’s native, fully on‑chain CLOB. It introduces features like flash loans, governance, and improved account abstraction, while retaining low‑latency matching on Sui. Orders are placed against pools with defined tick, lot, and minimum size constraints; a BalanceManager shared object holds balances for an account and controls order permissions. Fees can be paid in the DEEP token when pay_with_deep=true. Numi’s adapter respects these constraints and maintains per‑pool quantizers so orders are always valid on‑chain.  ￼

For reads, Numi uses the DeepBookV3 Indexer, which exposes real‑time order book levels, pool metadata, and historical signals. This avoids polling raw objects and keeps quote staleness bounded to the indexer’s streaming freshness.  ￼

⸻

4. Architecture Overview

Design goals. Numi targets sub‑second confirmations for fast‑path routes and deterministic behavior under contention. It partitions responsibilities across three planes to control complexity and enable independent scaling.

Execution plane. The router compiles selected strategies into PTBs that prefer owned‑state mutations. It selects validators based on rolling effects‑latency telemetry and submits through the Transaction Driver. Retries are idempotent by transaction digest, avoiding duplicate effects. Sponsored transactions eliminate user gas friction for priority flows.  ￼

Data plane. All hot reads use gRPC streams (checkpoints, effects, objects). Structured queries and historical joins use the GraphQL RPC + Indexer, which processes checkpoints into a relational model and can connect to archival stores. The DeepBook Indexer supplies pool parameters and L2 snapshots for quoting and compliance checks.  ￼

Control plane. Admission control, sponsorship budgets, key management, and rate limits live here. Controls are driven by service‑level indicators—e.g., quote staleness, route compute time, submission RTT, effects time, and success rates per route class.

⸻

5. Execution Model and Latency Budget

Fast‑path dominance. The router penalizes routes that require shared‑object contention unless price improvement offsets expected latency. This is optimal on Sui because owned‑object transactions skip the consensus wait, while shared‑object transactions pay the Mysticeti latency budget.  ￼

Validator selection and placement. With Mysticeti v2 and the Transaction Driver, Sui reduces round trips relative to the legacy Quorum Driver. Numi keeps connections warm to multiple validators, uses an EWMA of observed effects times to choose the next submit target, and colocates compute in regions with consistently low end‑to‑end times.  ￼

PTB composition. Single‑leg orders, multi‑venue split routes, flash‑loan backed arbs, and cancel/replace chains are encoded as PTBs. The 1,024‑command ceiling leaves headroom for complex routes while retaining atomic failure semantics and predictable gas.  ￼

Sponsored transactions. For user experience and throughput, Numi supports sponsorship paths where the platform funds gas and enforces budgets and abuse heuristics. Sponsored transactions are first‑class in Sui and can be built from transaction‑kind bytes or full PTBs.  ￼

⸻

6. Data Access and Consistency

gRPC as primary. The full node gRPC API is optimized for performance and typed payloads. It supports streaming subscription to checkpoints and object updates, which Numi uses to refresh synthetic books and reconcile state.  ￼

GraphQL RPC + General‑Purpose Indexer. Numi uses GraphQL RPC for structured queries and historical joins, backed by the general‑purpose indexer that transforms checkpoints into a relational store. An archival connector provides access to historical state without custom pipelines. This reduces bespoke ETL and simplifies compliance queries.  ￼

DeepBook Indexer for market data. For pool metadata and order books, the DeepBook Indexer gives an authoritative, consistent stream designed specifically for the CLOB’s semantics.  ￼

⸻

7. On‑Chain Patterns That Make Numi Fast

Owned‑state first. When feasible, Numi structures per‑user state and per‑strategy data as address‑owned objects. These transactions finalize on the fast path and don’t contend on global ordering, which keeps p99 in check. Shared‑object actions are isolated to minimal steps that require ordering.  ￼

Dynamic fields and limits awareness. Where dynamic collections are needed, Numi respects protocol limits (e.g., event and dynamic‑field access counts) and designs batch sizes and PTBs accordingly to avoid aborts under load.  ￼

Checkpoints as the source of truth. Sui forms checkpoints after execution to certify history. Numi reconciles state changes against checkpoint streams for provable inclusion and clean recovery on restarts.  ￼

⸻

8. DeepBook V3 Order Lifecycle in Numi

Pre‑trade checks. The router validates BalanceManager funding, quantizes price and size to tick/lot/min constraints, and ensures fee‑payment configuration (DEEP or input asset) is correct.  ￼

Route selection. Price‑of‑execution includes L2 price, expected fill slippage, gas, venue failure risk, and a measured latency penalty for shared‑object routes. The router prefers DeepBook when CLOB depth is favorable, or composes hybrid routes with AMMs when path liquidity and slippage beat CLOB‑only.

Placement and confirmation. The route compiles into a PTB using DeepBook’s SDK semantics, then submits through the Transaction Driver. The client surfaces digest, effects, and the book snapshot used for placement.

⸻

9. Security, Keys, and Sponsorship Controls

Key management. Ed25519 or other supported schemes are maintained in HSM/KMS. Transactions use Sui’s intent signing model, and sponsorship flows isolate sponsor keys and budgets from user flows.  ￼

Sponsored transaction risk controls. The platform sets per‑user and per‑route caps and actively prevents equivocation scenarios in which owned objects could become locked when competing transactions are signed concurrently. Numi enforces serialization where needed and uses PTBs to coalesce multi‑step operations safely.  ￼

Operational guardrails. Admission control sheds load on shared‑object venues during congestion. Circuit breakers halt risky strategies upon deviations in effects time, failure rate, or quote staleness.

⸻

10. Reliability and Observability

Failure domains. Numi isolates the hot execution path from the data and control planes. If the DeepBook Indexer stalls, the router downgrades to local synthetic books with explicit freshness indicators. If a validator degrades, the Transaction Driver selector rotates immediately to the next healthy candidate.  ￼

Telemetry. End‑to‑end traces span: route compute, submit RTT, effects time, checkpoint inclusion delay, and post‑trade reconciliation. These feed SLOs tied to user‑visible outcomes rather than raw throughput alone.

⸻

11. Benchmarks and Targets

Targets. The design aims for sub‑second confirmations for owned‑state routes and competitive latencies for shared‑object routes even under load. Mysticeti’s integration and the Transaction Driver reduce the overhead of ordering and certification compared with prior stacks; Sui reports sub‑second consensus latencies and public measurements around ~390 ms in steady state, providing a practical baseline for p50 confirmations when routes avoid shared contention. Actual numbers depend on placement, validator health, and venue mix; Numi measures and adapts in real time.  ￼

Throughput. PTBs let a single user action trigger many atomic steps, which improves effective throughput by collapsing multi‑stage flows into one ordered commit. The 1,024‑command ceiling bounds worst‑case batch sizes and simplifies gas modeling.  ￼

⸻

12. Compliance, Auditability, and Data Provenance

Deterministic state proofs. Numi records transaction digests, effects, and checkpoint sequence numbers for all executed routes. Because checkpoints certify chain history after execution, the platform can prove inclusion and reconstruct market‑state decisions ex post.  ￼

Indexing hygiene. The GraphQL RPC + Indexer reduces custom ETL, which lowers operational risk and makes audits repeatable against a standard schema and archival source.  ￼

⸻

13. 
	Architected for Sui’s strengths, not around them. Numi explicitly optimizes for the fast path, keeping the majority of updates off consensus and reserving Mysticeti’s capacity for operations that require ordering. This is not a generic EVM‑style router port; it is purpose‑built for Sui’s object model.  ￼
	2.	Modern, typed data plane. By adopting gRPC and GraphQL RPC + Indexer now, Numi stays aligned with Sui’s deprecation timeline, avoids legacy JSON‑RPC corner cases, and gains streaming plus strongly typed queries.  ￼
	3.	Native CLOB with a first‑party indexer. DeepBook V3 plus its indexer allow consistent pricing and verification without bespoke pipelines, and the BalanceManager abstraction cleanly separates funding, permissions, and trading.  ￼
	4.	Consensus built for latency. Mysticeti v2 and the Transaction Driver reduce end‑to‑end confirmation overhead; Numi’s validator selection and colocation strategy capture those gains in practice.  ￼
	5.	Atomic multi‑venue routes. PTBs provide true on‑chain atomicity across many steps with clear failure semantics and controlled limits, a better fit than multi‑transaction orchestration for high‑frequency flows.  ￼
	6.	Gasless UX at scale. Sponsored transactions are first‑class in Sui; Numi integrates sponsorship budgeting and abuse detection to remove gas friction without sacrificing safety.  ￼

⸻

14. Roadmap
	•	Full gRPC execution everywhere. As providers standardize Transaction Execution gRPC, flip all submit paths to typed gRPC and retire JSON‑RPC ahead of the April 2026 deadline.  ￼
	•	Deeper GraphQL adoption. Migrate compliance and analytics to GraphQL RPC + Indexer, using archival connectors for long‑range replay.  ￼
	•	Advanced venue adapters. Extend the router with additional Sui venues and specialized order types, maintaining BalanceManager hygiene and shared‑object isolation.  ￼
	•	Adaptive latency routing. Incorporate live consensus health signals (e.g., Mysticeti round metrics) into route scoring to price congestion more precisely.  ￼

⸻

15. Conclusion

Numi Sui Stack represents a deliberate fusion of Sui’s hybrid execution model, DeepBook V3’s native CLOB, and a modern data plane that abandons legacy JSON‑RPC. By privileging owned‑object fast‑path design, using PTBs for atomic multi‑venue composition, and executing through the Transaction Driver alongside Mysticeti v2, Numi delivers a platform prepared for competitive latency and clean scalability. The result is a world‑class aggregator and execution engine tailored to Sui’s unique primitives, with verifiable performance, auditable state, and a clear path to continued improvement as Sui’s gRPC and GraphQL ecosystem matures.  ￼

⸻

References
	•	Sui gRPC overview and deprecation timeline for JSON‑RPC (April 2026).  ￼
	•	GraphQL RPC + General‑Purpose Indexer (Beta) overview.  ￼
	•	Sui object model, address‑owned vs shared, and fast‑path details.  ￼
	•	Mysticeti v2 and Transaction Driver performance notes.  ￼
	•	PTB semantics and 1,024‑operation limit.  ￼
	•	DeepBook V3 overview, orders, BalanceManager; DeepBook Indexer.  ￼
	•	Sui checkpoints and certified history after execution.  ￼
	•	Sponsored transactions concepts and patterns.  ￼

written by AI 