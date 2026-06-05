//! Built-in conversation templates derived from ShareGPT-style multi-turn patterns.
//!
//! Each template has a seed (first user turn) and a pool of follow-ups that
//! stay within the same topic. The sampler picks a random template per
//! conversation and samples follow-ups without immediate repeats, producing
//! realistic cache-busting traffic that naturally grows toward the context limit.

use rand::SeedableRng;
use rand::rngs::SmallRng;
use rand::seq::IndexedRandom;

pub struct ConvTemplate {
    pub seed: &'static str,
    pub followups: &'static [&'static str],
}

pub struct PromptSampler {
    rng: SmallRng,
    template: &'static ConvTemplate,
    system: &'static str,
    last: Option<usize>,
}

impl PromptSampler {
    /// Create a sampler with a random template and random system prompt.
    pub fn new() -> Self {
        let mut rng = SmallRng::from_os_rng();
        let template = TEMPLATES.choose(&mut rng).unwrap();
        let system = SYSTEM_PROMPTS.choose(&mut rng).copied().unwrap_or("");
        Self { rng, template, system, last: None }
    }

    /// System prompt for this conversation (randomly chosen at construction).
    pub fn system(&self) -> &'static str {
        self.system
    }

    /// First user turn for this conversation.
    pub fn seed(&self) -> &'static str {
        self.template.seed
    }

    /// Subsequent user turns — random follow-up within the same category,
    /// never the same index twice in a row.
    pub fn next(&mut self) -> &'static str {
        let pool = self.template.followups;
        let len = pool.len();
        loop {
            let idx = (0..len).collect::<Vec<_>>().choose(&mut self.rng).copied().unwrap_or(0);
            if Some(idx) != self.last {
                self.last = Some(idx);
                return pool[idx];
            }
        }
    }
}

pub static TEMPLATES: &[ConvTemplate] = &[

    // ── Python web scraping ───────────────────────────────────────────────────
    ConvTemplate {
        seed: "I want to build a production-grade web scraper in Python to collect product prices and availability from multiple e-commerce sites. What architecture should I use?",
        followups: &[
            "Show me a complete implementation using aiohttp and BeautifulSoup with async concurrency and a configurable rate limiter.",
            "How do I handle JavaScript-rendered pages? Give me a full Playwright-based solution that integrates with the async scraper.",
            "Add rotating proxy support with automatic ban detection and fallback logic.",
            "Implement a persistent job queue using Redis so scraping can be distributed across multiple workers.",
            "How do I parse inconsistent HTML where price formats differ across sites? Show me a robust normalization pipeline.",
            "Add structured logging, per-site metrics, and a Prometheus endpoint to the scraper.",
            "Implement a retry strategy with exponential backoff and circuit breaker per domain.",
            "How do I store the scraped data in PostgreSQL with proper schema design for time-series price tracking?",
            "Add a change-detection layer so we only store records when the price actually changes.",
            "How do I deploy this scraper as a Kubernetes CronJob with proper resource limits?",
            "Write a test suite that mocks HTTP responses and validates parsing logic across fixture HTML files.",
            "Implement respect for robots.txt and crawl-delay headers without sacrificing throughput.",
            "How do I handle CAPTCHAs gracefully — detect, back off, and alert without crashing the pipeline?",
            "Add deduplication so that concurrent workers don't process the same URL twice.",
            "Show me how to build a simple dashboard in Grafana that shows scrape success rates and price trends.",
        ],
    },

    // ── REST API design ───────────────────────────────────────────────────────
    ConvTemplate {
        seed: "I need to design a REST API for a multi-tenant SaaS platform that handles billing, user management, and resource provisioning. Where do I start?",
        followups: &[
            "Design the full URL hierarchy and HTTP verb mapping for tenants, users, subscriptions, and resources.",
            "How should I handle API versioning? Give me a concrete strategy with migration examples.",
            "Implement JWT-based authentication with refresh tokens, revocation, and per-tenant key rotation.",
            "Design the rate limiting strategy: per-user, per-tenant, and global tiers with proper 429 responses and Retry-After headers.",
            "How do I model and return partial failures when a batch operation affects multiple resources?",
            "Implement cursor-based pagination that works correctly under concurrent inserts and deletes.",
            "Design an idempotency system so clients can safely retry POST requests without duplicate side effects.",
            "How should I handle long-running operations — polling vs webhooks vs Server-Sent Events?",
            "Write an OpenAPI 3.1 spec for the core endpoints including all error schemas.",
            "How do I implement field-level filtering and sparse fieldsets without leaking cross-tenant data?",
            "Design the audit log API so tenants can query every mutation to their resources.",
            "Implement request signing (HMAC-SHA256) for machine-to-machine API clients.",
            "How do I design consistent error responses across all endpoints including validation errors?",
            "Add a webhook delivery system with signing, retries, and a replay endpoint.",
            "How should the API behave during planned maintenance — 503 with Retry-After or degraded mode?",
        ],
    },

    // ── Database optimization ─────────────────────────────────────────────────
    ConvTemplate {
        seed: "Our PostgreSQL database is struggling under load — queries that used to take 10ms now take 3 seconds at peak traffic. How do I systematically diagnose and fix this?",
        followups: &[
            "Walk me through reading and interpreting EXPLAIN ANALYZE output for a slow query with nested loops.",
            "How do I identify which queries are causing the most total load using pg_stat_statements?",
            "Design the optimal index strategy for a table with 50M rows queried by user_id, created_at, and status.",
            "Explain partial indexes, expression indexes, and covering indexes — when should I use each?",
            "How do I detect and fix table bloat from dead tuples and a misconfigured autovacuum?",
            "Our joins are slow. When should I denormalize, and how do I do it without breaking data integrity?",
            "Implement a read replica strategy with application-level routing that handles replication lag safely.",
            "How do I partition a large table by date range without downtime on a live system?",
            "Explain connection pooling with PgBouncer — transaction vs session mode trade-offs and configuration.",
            "How do I use materialized views for expensive aggregations and keep them fresh without locking?",
            "Implement row-level security in PostgreSQL for our multi-tenant data model.",
            "How do I safely add NOT NULL columns and new indexes to a 100M-row table with zero downtime?",
            "Design a caching layer with Redis in front of PostgreSQL that handles cache invalidation correctly.",
            "How do I write a query planner hint or rewrite a query to avoid a bad plan without changing the data model?",
            "What should I monitor in Grafana to detect database degradation before users notice?",
        ],
    },

    // ── React frontend ────────────────────────────────────────────────────────
    ConvTemplate {
        seed: "I'm building a complex React application for real-time collaborative document editing. What state management and architecture choices should I make?",
        followups: &[
            "Compare Zustand, Jotai, Redux Toolkit, and React Query for this use case — recommend one and show the setup.",
            "Implement real-time collaboration using WebSockets with operational transforms to handle concurrent edits.",
            "How do I structure the component hierarchy to avoid prop drilling while keeping performance good?",
            "Implement virtualized rendering for documents with thousands of paragraphs using react-window.",
            "How do I handle optimistic updates so the UI feels instant while mutations are in flight?",
            "Write a custom hook that manages WebSocket connection lifecycle with reconnection and backoff.",
            "How do I implement undo/redo across collaborative sessions without conflicts?",
            "Add offline support using a service worker and IndexedDB so users can edit without connectivity.",
            "Profile and fix re-render performance — show me how to use React DevTools Profiler and useMemo correctly.",
            "Implement code splitting and lazy loading so the initial bundle stays under 150kb.",
            "How do I test components that depend on WebSocket state using React Testing Library?",
            "Add accessibility (ARIA roles, keyboard navigation) to the collaborative editor.",
            "Implement a presence system showing which users are viewing and editing each section.",
            "How do I handle authentication token refresh without interrupting real-time collaboration?",
            "Set up error boundaries that degrade gracefully so one component crash doesn't lose user work.",
        ],
    },

    // ── Machine learning pipeline ─────────────────────────────────────────────
    ConvTemplate {
        seed: "I need to build an end-to-end ML pipeline to predict customer churn for a SaaS product. Walk me through the full process from raw data to production model.",
        followups: &[
            "What features should I engineer from event logs, billing data, and support tickets? Show me the feature store design.",
            "Walk me through exploratory data analysis — what distributions and correlations should I check first?",
            "Our churn dataset is heavily imbalanced (5% positive). What techniques should I use and what are their trade-offs?",
            "Compare gradient boosting (XGBoost, LightGBM) vs neural networks for tabular churn prediction.",
            "Implement a full training pipeline with cross-validation, hyperparameter tuning, and experiment tracking in MLflow.",
            "How do I evaluate the model beyond accuracy? Show me precision-recall curves, calibration, and business metrics.",
            "Explain SHAP values and show me how to use them to explain individual predictions to account managers.",
            "How do I detect and handle data leakage in a time-series churn problem?",
            "Design the model serving layer: batch inference vs real-time scoring trade-offs and implementation.",
            "How do I monitor the model in production for data drift and concept drift?",
            "Implement A/B testing to measure whether acting on churn predictions actually reduces churn.",
            "How do I retrain the model on a schedule with automated data validation and rollback if quality drops?",
            "Design the feature pipeline with proper train/validation/test splits that respect temporal ordering.",
            "How do I handle missing data — imputation strategies and when to use indicator variables?",
            "Show me how to build a Grafana dashboard tracking model performance and business outcomes over time.",
        ],
    },

    // ── Docker & Kubernetes ───────────────────────────────────────────────────
    ConvTemplate {
        seed: "We're migrating a monolithic Python Django application to Kubernetes. What's the right migration strategy and how do I containerize it properly?",
        followups: &[
            "Write a production-grade multi-stage Dockerfile for Django that minimises image size and attack surface.",
            "How do I handle database migrations safely in Kubernetes — init containers vs migration jobs?",
            "Design the Kubernetes manifests: Deployment, Service, Ingress, ConfigMap, and Secret for the Django app.",
            "Implement horizontal pod autoscaling based on custom metrics like request queue depth.",
            "How do I configure liveness, readiness, and startup probes correctly so rolling updates don't cause downtime?",
            "Set up a Helm chart for the application with environment-specific values files.",
            "How do I handle static and media files in Kubernetes — S3 with CloudFront vs shared volumes?",
            "Implement resource requests and limits — how do I right-size them without starving or wasting?",
            "How do I run Celery workers alongside the Django app with proper scaling and shutdown handling?",
            "Set up network policies to restrict pod-to-pod communication to only what's needed.",
            "How do I configure secrets management using Vault or Kubernetes External Secrets?",
            "Implement a canary deployment strategy so I can roll out to 5% of traffic first.",
            "How do I set up cluster autoscaling to handle traffic spikes without over-provisioning?",
            "Design a GitOps workflow with ArgoCD so every merge to main deploys automatically.",
            "How do I debug a pod that's crashing — walk me through the full diagnostic process.",
        ],
    },

    // ── System design ─────────────────────────────────────────────────────────
    ConvTemplate {
        seed: "Design a URL shortening service like bit.ly that needs to handle 100,000 redirects per second globally with sub-10ms latency. Walk me through the full system.",
        followups: &[
            "How do I generate short codes that are collision-free at scale without a coordination bottleneck?",
            "Design the data model — what do I store, how do I shard the database, and what's the partition key?",
            "How do I cache aggressively with Redis while handling cache invalidation when a URL is deleted?",
            "Design the global CDN and edge caching strategy for redirect responses.",
            "How does the system handle 10× traffic spikes — where are the bottlenecks and how do I eliminate them?",
            "Implement analytics: tracking clicks, referrers, and geographic distribution without slowing down redirects.",
            "How do I detect and block malicious URLs (phishing, malware) before they go live?",
            "Design the API for programmatic access with rate limiting per API key.",
            "How do I handle custom vanity slugs while keeping the short code namespace clean?",
            "Walk me through the failure modes — what happens if the database is down, if Redis is unavailable?",
            "Design a multi-region active-active setup with conflict resolution for concurrent writes.",
            "How do I implement URL expiration and cleanup at scale without locking the main database?",
            "What does the monitoring and alerting setup look like — what SLOs would you set?",
            "How do I implement a QR code generation feature without adding latency to the core redirect path?",
            "Walk me through capacity planning: storage, bandwidth, and compute for 1 billion links.",
        ],
    },

    // ── Rust systems programming ──────────────────────────────────────────────
    ConvTemplate {
        seed: "I'm learning Rust coming from C++ and I want to build a high-performance TCP server. Help me understand the ownership model and how to structure the project.",
        followups: &[
            "Explain lifetimes with a concrete example involving a struct that holds references.",
            "How do I structure async Rust with Tokio — tasks, channels, and shared state?",
            "Implement a connection pool that limits concurrent connections and times out idle ones.",
            "How do I handle backpressure in an async Rust TCP server under high load?",
            "Explain the difference between Arc<Mutex<T>>, Arc<RwLock<T>>, and lock-free alternatives.",
            "Implement a zero-copy parser for a custom binary protocol using nom or manual byte slicing.",
            "How do I write idiomatic error handling in Rust — when to use thiserror vs anyhow vs custom types?",
            "Profile the server with perf and flamegraph — walk me through finding and fixing a hot path.",
            "How do I implement graceful shutdown that drains in-flight requests without dropping connections?",
            "Write property-based tests with proptest for the protocol parser.",
            "How do I use unsafe Rust correctly — when is it justified and how do I audit it?",
            "Implement a simple memory allocator in Rust to understand how allocations work.",
            "How do I cross-compile the server for ARM and embed it on a Raspberry Pi?",
            "Explain how Rust's async runtime works under the hood — futures, polls, and wakers.",
            "How do I benchmark the server with criterion and avoid common microbenchmark pitfalls?",
        ],
    },

    // ── Go microservices ──────────────────────────────────────────────────────
    ConvTemplate {
        seed: "I'm building a payment processing microservice in Go. It needs to be highly reliable, handle idempotency, and integrate with Stripe. How do I structure this?",
        followups: &[
            "Design the service structure — packages, dependency injection, and interface boundaries.",
            "Implement idempotent payment creation using an idempotency key stored in PostgreSQL.",
            "How do I handle Stripe webhook delivery — signature verification, deduplication, and ordered processing?",
            "Design the saga pattern for a checkout flow that spans three services: inventory, payment, and fulfillment.",
            "How do I implement circuit breakers and retries for Stripe API calls using a library like go-resiliency?",
            "Write the gRPC proto definition and server implementation for internal service-to-service calls.",
            "How do I test the payment service with Stripe's test mode and testcontainers for PostgreSQL?",
            "Implement distributed tracing with OpenTelemetry so I can see the full checkout flow in Jaeger.",
            "How do I handle currency and decimal arithmetic correctly in Go to avoid floating point errors?",
            "Design the reconciliation job that detects discrepancies between Stripe and our database.",
            "How do I structure Go error handling so payment failures are classified and alerting is meaningful?",
            "Implement a dead-letter queue for failed webhook events with manual replay capability.",
            "How do I do zero-downtime deployments of the payment service without dropping in-flight transactions?",
            "Write a load test with k6 that simulates realistic payment traffic including bursts.",
            "How do I implement PCI DSS compliance requirements in the codebase — what must I never log or store?",
        ],
    },

    // ── Distributed systems ───────────────────────────────────────────────────
    ConvTemplate {
        seed: "Explain the Raft consensus algorithm to me — I need to understand it deeply enough to implement it in my own distributed key-value store.",
        followups: &[
            "Walk me through leader election in detail — what happens when a follower times out and triggers an election?",
            "How does log replication work — what guarantees does Raft make about which entries are committed?",
            "Explain the edge cases: split votes, network partitions, and leader crashes mid-replication.",
            "How do I implement log compaction with snapshots so the log doesn't grow forever?",
            "What is linearizability and how does Raft achieve it?",
            "Implement the Raft state machine in Python or Go — show me the core data structures and message types.",
            "How do I add cluster membership changes (adding/removing nodes) without violating safety?",
            "Explain how read scaling works — follower reads vs lease-based reads.",
            "What are the performance trade-offs between Raft and Paxos? When would you choose one over the other?",
            "How does etcd implement Raft and what optimisations does it add beyond the basic algorithm?",
            "Walk me through testing a distributed system — how do I inject network partitions and crashes in tests?",
            "How do I monitor a Raft cluster — what metrics indicate an unhealthy leader or lagging follower?",
            "Implement a simple key-value store on top of the Raft log with a snapshot mechanism.",
            "How does CockroachDB use Raft at the per-range level to build a globally distributed SQL database?",
            "What are the latency implications of Raft at geographically distributed nodes and how do you mitigate them?",
        ],
    },

    // ── Security hardening ────────────────────────────────────────────────────
    ConvTemplate {
        seed: "Our startup just got a security audit back and we have a list of critical findings. Help me prioritise and fix them systematically.",
        followups: &[
            "We have SQL injection vulnerabilities in legacy PHP code. Show me how to remediate them with parameterised queries and a WAF.",
            "Our JWT tokens never expire and there's no revocation mechanism. Design a proper token lifecycle.",
            "We're storing passwords with MD5. Walk me through migrating to Argon2id without invalidating existing user sessions.",
            "We have IDOR vulnerabilities where users can access other users' data by changing IDs. How do I fix this at the framework level?",
            "Our S3 buckets are publicly readable. Audit and harden the IAM policies and bucket ACLs.",
            "We have no rate limiting and are getting credential stuffing attacks. Implement defence in depth.",
            "Explain how to implement Content Security Policy to prevent XSS — show me the header values for a React SPA.",
            "Our API keys are committed in git history. Walk me through rotating them and preventing future leaks.",
            "We have no audit logging. Design a tamper-evident audit log system for all sensitive operations.",
            "Implement SSRF protection in a feature that fetches URLs provided by users.",
            "How do I implement proper CORS — explain the preflight flow and show me a safe configuration.",
            "We're running outdated dependencies with known CVEs. Design a process for staying current.",
            "How do I implement MFA correctly — TOTP setup, backup codes, and recovery flows?",
            "Design a secrets management strategy using Vault to eliminate environment variable secrets.",
            "How do I set up a responsible disclosure programme and handle incoming vulnerability reports?",
        ],
    },

    // ── Data engineering ──────────────────────────────────────────────────────
    ConvTemplate {
        seed: "I need to build a real-time data pipeline that ingests clickstream events from our web app, enriches them, and makes them available for analytics within 30 seconds. Design the architecture.",
        followups: &[
            "Design the Kafka topic schema and partitioning strategy for clickstream events.",
            "Implement a Flink job that joins clickstream events with a user profile lookup in real time.",
            "How do I handle late-arriving events — windowing strategies and watermarks in streaming systems?",
            "Design the schema evolution strategy so new event fields don't break downstream consumers.",
            "How do I implement exactly-once processing end-to-end from Kafka through Flink to the data warehouse?",
            "Compare Delta Lake, Apache Iceberg, and Apache Hudi for our real-time analytics use case.",
            "Implement data quality checks in the pipeline — what do I validate and how do I handle failures?",
            "How do I partition the output data in S3/GCS for optimal query performance in Athena or BigQuery?",
            "Design the backfill strategy to reprocess historical data when the enrichment logic changes.",
            "How do I monitor pipeline lag and alert before it exceeds our 30-second SLO?",
            "Implement PII detection and redaction in the pipeline to comply with GDPR.",
            "Design a dead-letter queue for malformed events with a replay and inspection UI.",
            "How do I test a streaming pipeline — unit tests for transformations and integration tests with embedded Kafka?",
            "Compare batch processing with Spark vs streaming with Flink — when does each make sense?",
            "How do I implement cost optimisation — compression, compaction, and lifecycle policies?",
        ],
    },

    // ── Frontend performance ──────────────────────────────────────────────────
    ConvTemplate {
        seed: "Our Next.js e-commerce site has a Lighthouse performance score of 38 on mobile. Walk me through a systematic approach to improve it to 90+.",
        followups: &[
            "Explain Core Web Vitals — LCP, CLS, and INP — and show me how to measure each one in production.",
            "Our LCP is 6.2 seconds. Walk me through diagnosing whether it's network, render-blocking resources, or slow server response.",
            "Implement image optimisation: next/image, WebP conversion, responsive srcsets, and lazy loading.",
            "How do I eliminate render-blocking JavaScript and CSS — what does the optimal critical path look like?",
            "Our JavaScript bundle is 2.4MB. Show me how to analyse it with webpack-bundle-analyzer and split it aggressively.",
            "Implement a service worker for aggressive caching of static assets with a stale-while-revalidate strategy.",
            "How do I optimise web fonts to prevent FOIT and FOUT — font-display, preload, and subsetting.",
            "Our server response time (TTFB) is 800ms. Diagnose whether it's the database, network, or application code.",
            "Implement efficient code splitting in Next.js — per-route and per-component lazy loading.",
            "How do I detect and fix layout shift — give me a checklist for CLS common causes.",
            "Add resource hints: preconnect, dns-prefetch, preload, and prefetch — where does each one belong?",
            "How do I implement a CDN strategy for both static assets and edge-rendered pages?",
            "Measure real user performance with web-vitals.js and send metrics to a custom dashboard.",
            "How do I optimise third-party scripts (analytics, chat widgets) so they don't tank performance?",
            "Implement server-side rendering vs static generation — decision framework and implementation for each page type.",
        ],
    },

    // ── Algorithm design ──────────────────────────────────────────────────────
    ConvTemplate {
        seed: "I'm preparing for FAANG-level algorithm interviews. Start me with a hard graph problem and explain the solution thoroughly.",
        followups: &[
            "Now walk me through Dijkstra's algorithm — implementation, complexity, and when to use Bellman-Ford instead.",
            "Explain topological sort and show me how to detect cycles in a directed graph.",
            "Walk me through the A* search algorithm and explain how the heuristic function affects performance.",
            "Give me a hard dynamic programming problem involving intervals and walk me through the optimal substructure.",
            "Explain the sliding window technique — show me three problems of increasing difficulty.",
            "How do segment trees work? Implement range sum and range minimum queries with lazy propagation.",
            "Walk me through the union-find data structure and show me a problem where it's the elegant solution.",
            "Explain binary search on the answer — give me three problems where the search space isn't an array.",
            "How do I implement a trie and when is it better than a hash map?",
            "Give me a hard two-pointer problem and explain how to arrive at the solution systematically.",
            "Walk me through minimum spanning trees — Kruskal vs Prim and when each is preferable.",
            "Explain network flow and the Ford-Fulkerson algorithm — give me a real interview problem that reduces to max flow.",
            "How do I approach bit manipulation problems — show me the common tricks with examples.",
            "Walk me through the KMP string matching algorithm — why is it O(n+m) and how does the failure function work?",
            "Give me a hard interval scheduling problem and show me the greedy proof of correctness.",
        ],
    },

    // ── Refactoring legacy code ───────────────────────────────────────────────
    ConvTemplate {
        seed: "We inherited a 10-year-old PHP monolith with no tests, mixed concerns everywhere, and several critical features nobody understands. How do I modernise it without breaking production?",
        followups: &[
            "How do I add a test harness to untested legacy code? Walk me through the characterisation test approach.",
            "Explain the strangler fig pattern — how do I incrementally replace the monolith with services?",
            "Our database has no foreign keys, nullable everything, and business logic in stored procedures. Where do I start?",
            "How do I identify and safely extract a self-contained bounded context without breaking the rest of the system?",
            "Implement a feature flag system so I can deploy the new code alongside the old and switch traffic gradually.",
            "How do I handle the shared database problem when splitting a monolith — the data is deeply coupled?",
            "Walk me through refactoring a 1500-line God class without changing its external behaviour.",
            "How do I establish code ownership and prevent further decay while the team is under feature pressure?",
            "Design an event bus so the monolith can emit domain events that new services can consume.",
            "How do I safely rename a database column that is referenced in 200 places across the codebase?",
            "Implement contract tests between the monolith and the first extracted service.",
            "How do I measure the health of the refactoring — what metrics tell me things are getting better?",
            "What do I do when a critical bug is discovered in code nobody understands — how do I fix it safely?",
            "How do I manage the dual-write period where both old and new code must update the same data?",
            "Design a migration plan that keeps the business shipping features while paying down this technical debt.",
        ],
    },

    // ── Cloud architecture AWS ────────────────────────────────────────────────
    ConvTemplate {
        seed: "We're building a new product on AWS and need to design a secure, scalable, cost-efficient architecture. We expect to grow from 0 to 1M users in 18 months.",
        followups: &[
            "Design the VPC architecture with public and private subnets across three availability zones.",
            "Compare ECS Fargate vs EKS vs Lambda for our stateless API services — recommend one and justify it.",
            "How do I implement infrastructure as code with Terraform — project structure, state management, and modules?",
            "Design the RDS architecture: multi-AZ, read replicas, and automated failover strategy.",
            "How do I implement least-privilege IAM policies — what does the role design look like for each service?",
            "Design the CDN and caching strategy with CloudFront for both static assets and API responses.",
            "How do I use AWS Secrets Manager and Parameter Store correctly — when to use each?",
            "Implement a CI/CD pipeline with CodePipeline or GitHub Actions that deploys to multiple environments.",
            "Design the observability stack: CloudWatch, X-Ray, and when to add a third-party tool.",
            "How do I implement DDoS protection and WAF rules without breaking legitimate traffic?",
            "Design the cost monitoring and alerting setup — how do I catch runaway spend before it's a problem?",
            "How do I implement disaster recovery: RTO/RPO targets and the technical implementation to meet them?",
            "Compare SQS, SNS, EventBridge, and Kinesis for our event-driven architecture — when to use each?",
            "How do I right-size EC2 instances and use Savings Plans vs Reserved Instances vs Spot correctly?",
            "Design the data lake architecture on S3 with proper partitioning, access control, and lifecycle policies.",
        ],
    },

    // ── Testing strategies ─────────────────────────────────────────────────────
    ConvTemplate {
        seed: "Our engineering team writes almost no tests and we're afraid to refactor anything. How do I build a testing culture and a practical test suite from scratch?",
        followups: &[
            "What is the testing pyramid and how do I decide how many tests at each level?",
            "Show me how to write a good unit test — what makes a test valuable vs a test that's just noise?",
            "How do I test code that has external dependencies — mocking vs test doubles vs testcontainers?",
            "Implement integration tests for a REST API using a real database and in-process server.",
            "How do I write end-to-end tests with Playwright that are reliable and don't flake?",
            "What is property-based testing and show me examples where it catches bugs that example tests miss.",
            "How do I measure and improve code coverage meaningfully — what does 80% coverage actually tell you?",
            "Design a mutation testing setup to check whether our tests actually catch bugs.",
            "How do I test asynchronous code — timeouts, polling, and race conditions in tests.",
            "Implement contract testing between a frontend and a backend team using Pact.",
            "How do I test a distributed system — chaos engineering, fault injection, and resilience tests.",
            "Design a test data management strategy: factories, fixtures, and database seeding that scales.",
            "How do I set up a fast test pipeline in CI that gives feedback in under 5 minutes?",
            "What is TDD and when is it worth the discipline — show me a worked example.",
            "How do I enforce test quality — what goes in the PR review checklist for tests?",
        ],
    },

    // ── Natural language processing ───────────────────────────────────────────
    ConvTemplate {
        seed: "I want to build a semantic search engine for our internal knowledge base using embeddings and vector search. Walk me through the full architecture.",
        followups: &[
            "Explain how text embeddings work — what does it mean for similar text to be close in vector space?",
            "Compare embedding models: OpenAI ada-002, sentence-transformers, and Cohere — how do I choose?",
            "How do I chunk documents intelligently so semantic search returns coherent results?",
            "Implement the ingestion pipeline: parse PDFs and Notion pages, chunk, embed, and upsert into Pinecone.",
            "How does approximate nearest neighbour search work — HNSW, IVF, and trade-offs between recall and speed?",
            "Implement hybrid search that combines vector similarity with BM25 keyword ranking.",
            "How do I build a RAG system on top of the vector search so users can ask questions in natural language?",
            "How do I evaluate retrieval quality — precision@k, recall@k, and MRR for our use case?",
            "Implement re-ranking with a cross-encoder to improve the quality of top-k results.",
            "How do I handle multilingual content — embed and search across English, French, and German?",
            "Design the access control layer so users only see search results they have permission to read.",
            "How do I keep the index fresh — incremental updates when documents change or are deleted?",
            "Implement query expansion and spelling correction to improve recall for imprecise queries.",
            "How do I measure whether the semantic search is actually helping users find what they need?",
            "What are the cost trade-offs between self-hosted embedding models and API providers at scale?",
        ],
    },

    // ── Compiler design ───────────────────────────────────────────────────────
    ConvTemplate {
        seed: "I want to build a simple interpreted programming language as a learning project. Walk me through writing a lexer, parser, and interpreter from scratch.",
        followups: &[
            "Implement a lexer in Python that tokenises arithmetic expressions, strings, identifiers, and keywords.",
            "How does a recursive descent parser work? Build one for a grammar with operator precedence.",
            "Explain the difference between an AST and a CST — when do you need each?",
            "Implement a tree-walking interpreter that evaluates the AST including variables and functions.",
            "Add a type system — how do I implement static typing with type inference?",
            "How do I implement closures and lexical scoping correctly — show me the environment model.",
            "Implement garbage collection — reference counting vs mark-and-sweep, show me the simplest viable version.",
            "How do I add error recovery to the parser so it reports multiple errors instead of stopping at the first?",
            "Implement a bytecode compiler and a stack-based virtual machine for better performance.",
            "How do I add tail call optimisation to the interpreter so recursive programs don't stack overflow?",
            "Implement a REPL with history, multi-line input, and syntax highlighting.",
            "How do I add a standard library — what are the essential built-in functions and how do I expose them?",
            "Implement pattern matching — how does the compiler desugar complex patterns into simple conditionals?",
            "How do I implement generics — what is monomorphisation and when is type erasure preferable?",
            "Explain LLVM — how would I use it as a backend to compile my language to native code?",
        ],
    },

    // ── Linux administration ──────────────────────────────────────────────────
    ConvTemplate {
        seed: "I need to set up a production Linux server from scratch — harden it, configure it for a web application, and make it observable. Walk me through the full process.",
        followups: &[
            "What are the first 10 commands I should run on a fresh Ubuntu server before doing anything else?",
            "Configure SSH hardening: disable root login, key-only auth, and fail2ban for brute force protection.",
            "Set up a non-root user with sudo, configure UFW firewall rules, and explain each decision.",
            "How do I configure systemd to run the application as a service with automatic restart and resource limits?",
            "Set up Nginx as a reverse proxy with SSL termination, HTTP/2, and security headers.",
            "How do I implement log rotation, aggregation with journald, and shipping to a central log store?",
            "Implement automated security updates with unattended-upgrades without risking application breakage.",
            "How do I monitor the server with node_exporter and Prometheus — what metrics matter most?",
            "Set up alerting for disk full, high CPU, memory pressure, and application errors.",
            "How do I implement a backup strategy: what to back up, how often, and how to test restoration?",
            "Configure kernel parameters (sysctl) for a high-traffic web server.",
            "How do I diagnose a server that's under load — walk me through the USE method.",
            "Implement intrusion detection with auditd and alert on suspicious file system changes.",
            "How do I configure TLS correctly — cipher suites, HSTS, OCSP stapling, and certificate renewal?",
            "Set up a cron job strategy that's reliable, logged, and doesn't create zombie processes.",
        ],
    },

    // ── GraphQL ───────────────────────────────────────────────────────────────
    ConvTemplate {
        seed: "We're adding a GraphQL API on top of an existing REST backend and PostgreSQL database. What's the right architecture and how do I avoid common pitfalls?",
        followups: &[
            "Design the GraphQL schema for a social platform with users, posts, comments, and follows.",
            "How do I solve the N+1 query problem with DataLoader — show me the implementation pattern.",
            "Implement cursor-based pagination in GraphQL with the Relay connection spec.",
            "How do I handle authentication and per-field authorisation in GraphQL resolvers?",
            "Design the error handling strategy — GraphQL errors vs HTTP errors and partial success responses.",
            "How do I implement real-time subscriptions over WebSocket with proper connection management?",
            "Implement query complexity analysis to prevent expensive queries from taking down the server.",
            "How do I implement persisted queries to reduce payload size and enable CDN caching?",
            "Design the federation strategy with Apollo Federation for splitting the schema across teams.",
            "How do I version a GraphQL API — deprecation, field evolution, and breaking change management?",
            "Implement file upload in GraphQL — multipart requests and streaming to S3.",
            "How do I test a GraphQL API — unit tests for resolvers vs integration tests with a real schema?",
            "Implement caching at the resolver level and explain how it interacts with HTTP caching.",
            "How do I monitor GraphQL in production — tracing individual field resolvers and detecting slow queries?",
            "Compare code-first vs schema-first GraphQL — when to use each and tooling for both approaches.",
        ],
    },

    // ── Embedded systems ──────────────────────────────────────────────────────
    ConvTemplate {
        seed: "I want to build a battery-powered IoT sensor that measures temperature and humidity, stores data locally, and uploads it when WiFi is available. Walk me through the hardware and firmware design.",
        followups: &[
            "Compare ESP32, STM32, and Nordic nRF52840 for this use case — power consumption, connectivity, and toolchain.",
            "How do I implement deep sleep with a scheduled wake-up in ESP32 to maximise battery life?",
            "Write a FreeRTOS task that reads the DHT22 sensor and stores readings in SPIFFS with error handling.",
            "How do I implement over-the-air firmware updates safely on an embedded device?",
            "Design the data format for local storage — when does JSON make sense vs a binary format?",
            "How do I implement WiFi reconnection with exponential backoff and offline buffering?",
            "Implement TLS for the MQTT connection to the cloud broker on a resource-constrained device.",
            "How do I handle clock synchronisation using NTP and what happens when the clock drifts?",
            "Design the battery monitoring circuit and implement low-battery alerting in firmware.",
            "How do I debug an embedded device without a debugger — logging over UART and watchdog timers?",
            "Implement a bootloader that validates firmware integrity before booting.",
            "How do I write unit tests for embedded C code on the host machine without hardware?",
            "Design the cloud backend that ingests sensor data from thousands of devices.",
            "How do I implement power profiling — measure actual current draw and identify which code paths drain the battery?",
            "What security considerations are specific to IoT devices — secure boot, attestation, and key storage?",
        ],
    },

    // ── Serverless architecture ───────────────────────────────────────────────
    ConvTemplate {
        seed: "We want to build a document processing pipeline using serverless functions. Documents are uploaded by users, processed through several transformation steps, and results are stored for retrieval. Design the system.",
        followups: &[
            "Design the event-driven flow from S3 upload trigger through Lambda processing steps to DynamoDB.",
            "How do I handle long-running document processing that exceeds Lambda's 15-minute limit?",
            "Implement error handling and dead-letter queues so no document is lost even if processing fails.",
            "How do I manage shared state between Lambda invocations without a database for every tiny operation?",
            "Design the fan-out pattern for processing a document through 5 independent transformation steps in parallel.",
            "How do I implement Lambda provisioned concurrency to eliminate cold starts for latency-sensitive steps?",
            "Design the IAM execution roles for each Lambda function following least privilege.",
            "How do I manage Lambda dependencies efficiently — layers vs bundling vs container images?",
            "Implement distributed tracing across the entire serverless pipeline with X-Ray.",
            "How do I test serverless functions locally — LocalStack, SAM CLI, and integration test strategies.",
            "Design the cost model — how do I estimate and optimise Lambda and S3 costs at scale?",
            "How do I handle idempotency in Lambda functions that may be invoked more than once?",
            "Implement a status API so users can poll the processing status of their document.",
            "How do I handle versioning of Lambda functions and roll back a bad deployment safely?",
            "Design the security model: VPC configuration, secrets access, and preventing function-to-function injection.",
        ],
    },

    // ── Monitoring & observability ────────────────────────────────────────────
    ConvTemplate {
        seed: "Our system has outages that we only find out about from customer complaints. Design a proper observability stack so we detect and diagnose incidents before users do.",
        followups: &[
            "Explain the three pillars of observability — metrics, logs, and traces — and how they complement each other.",
            "Design the Prometheus metrics hierarchy: what to instrument at the system, service, and business level.",
            "How do I write useful alerting rules that are actionable and don't produce alert fatigue?",
            "Implement structured logging with correlation IDs so I can trace a request across 10 services.",
            "How do I use OpenTelemetry to instrument a polyglot microservices system consistently?",
            "Design a Grafana dashboard for a service: the four golden signals and SLO burn rate.",
            "How do I implement synthetic monitoring — uptime checks that simulate real user journeys?",
            "Implement log anomaly detection so unexpected error patterns alert without a fixed threshold.",
            "How do I set SLOs and error budgets — walk me through the calculation and alert design.",
            "Design on-call rotation, escalation policies, and runbooks so incidents are handled efficiently.",
            "How do I do a blameless post-mortem — structure, timeline reconstruction, and action items?",
            "Implement distributed tracing sampling strategy — what percentage to sample and how to preserve important traces?",
            "How do I monitor third-party dependencies — what to do when Stripe or AWS is degraded?",
            "Design the cost-efficient log retention strategy: hot/warm/cold storage and what to keep for how long?",
            "How do I implement chaos engineering safely — starting with game days and moving to automated fault injection?",
        ],
    },

    // ── Personal finance for developers ──────────────────────────────────────
    ConvTemplate {
        seed: "I'm a software engineer in my late 20s earning $130k. I have no savings and I'm living paycheck to paycheck. Help me build a financial foundation from scratch.",
        followups: &[
            "Walk me through building a zero-based budget — every dollar allocated before the month starts.",
            "What should I prioritise: emergency fund, high-interest debt, or retirement contributions?",
            "Explain index fund investing for someone with no finance background — what should I buy and why?",
            "How does a 401k match work and why is not contributing up to the match leaving free money on the table?",
            "What is the difference between a Roth and traditional 401k — which is better for my situation?",
            "How do I negotiate a raise? Give me a script and the research I need to do beforehand.",
            "Explain equity compensation: RSUs, ISOs, NSOs — what questions should I ask before joining a startup?",
            "How do I evaluate a job offer — total compensation beyond base salary?",
            "What are the tax implications of selling company stock and how do I avoid a surprise bill?",
            "When does it make financial sense to buy a home vs rent for a software engineer?",
            "How do I build an investment portfolio allocation that matches my risk tolerance and time horizon?",
            "What is an HSA and why do financial advisors call it the best retirement account most people ignore?",
            "How do I think about income diversification — side projects, open source, content creation?",
            "What insurance do I actually need as a single developer and what's a waste of money?",
            "How do I model financial independence — what net worth target lets me stop working if I want to?",
        ],
    },

    // ── Computer vision ───────────────────────────────────────────────────────
    ConvTemplate {
        seed: "I want to build an object detection system that identifies defects on a manufacturing assembly line in real time using a camera feed. Walk me through the design.",
        followups: &[
            "Compare YOLO, Faster R-CNN, and DETR for real-time industrial defect detection — which fits our constraints?",
            "How do I collect and label training data for defects when defect examples are rare?",
            "Implement data augmentation strategies specific to manufacturing inspection — lighting, angle, and texture variations.",
            "How do I fine-tune a pre-trained YOLOv8 model on our custom defect dataset?",
            "What metrics should I use beyond mAP — false negative rate is critical in defect detection.",
            "How do I optimise the model for inference on an NVIDIA Jetson at 30 FPS?",
            "Implement TensorRT quantisation to speed up inference without unacceptable accuracy loss.",
            "Design the real-time inference pipeline: camera capture, pre-processing, inference, and alert.",
            "How do I handle model drift when the product line changes and defect patterns shift?",
            "Implement active learning to efficiently label new defect types as they appear.",
            "How do I build a human-in-the-loop review interface for borderline detections?",
            "Design the data flywheel — how do production inferences become new training data?",
            "How do I implement anomaly detection as a complement to the classifier for unknown defect types?",
            "What is the calibration strategy — how do I set the confidence threshold to balance precision and recall for our line?",
            "Design the edge deployment and update strategy for 50 cameras across multiple factory sites.",
        ],
    },

    // ── Open source contribution ──────────────────────────────────────────────
    ConvTemplate {
        seed: "I want to start contributing to open source to grow my skills and reputation. How do I find the right project and make my first meaningful contribution?",
        followups: &[
            "How do I evaluate whether a project is healthy and worth investing time in?",
            "Walk me through reading a large unfamiliar codebase efficiently — where do I start?",
            "How do I set up a local development environment for a complex open source project?",
            "What makes a good first issue and how do I approach it without wasting the maintainer's time?",
            "How do I write a pull request that gets merged — commit messages, description, and scope?",
            "What is the etiquette around asking for review and following up without being annoying?",
            "How do I handle a maintainer's request for changes that I disagree with?",
            "How do I move from occasional contributor to trusted regular — what does that path look like?",
            "How do I start my own open source project and build a community around it?",
            "What are the legal considerations — licences, CLAs, and what I need to know before contributing?",
            "How do I contribute to a project in a language or framework I'm still learning?",
            "How do I write documentation that maintainers actually want?",
            "How do open source maintainers think about backwards compatibility and breaking changes?",
            "How do I handle burnout as an open source contributor or maintainer?",
            "How has open source contribution helped developers get jobs, and how do I make my contributions visible?",
        ],
    },

    // ── Scientific computing ──────────────────────────────────────────────────
    ConvTemplate {
        seed: "I'm a physicist who knows Python but has never used it for serious numerical computing. I need to simulate a system of coupled differential equations. Where do I start?",
        followups: &[
            "Explain the difference between scipy.integrate.odeint and solve_ivp — when do I use each?",
            "How do I choose a solver: RK45 vs DOP853 vs Radau for stiff vs non-stiff systems?",
            "Implement a simulation of the Lorenz attractor with visualisation and phase space plots.",
            "How do I handle numerical instability — detecting it and choosing step sizes?",
            "Explain vectorisation with NumPy — rewrite this slow loop to use array operations.",
            "When should I use Cython, Numba, or C extensions to speed up numerical code?",
            "How do I use JAX for automatic differentiation and GPU acceleration of my simulation?",
            "Implement a Monte Carlo simulation with proper random number generation and variance reduction.",
            "How do I parallelise a parameter sweep across thousands of simulations on a cluster?",
            "Explain the finite element method and when to use FEniCS or similar frameworks.",
            "How do I validate a numerical simulation — convergence testing and comparison with analytical solutions?",
            "Implement Fourier analysis of simulation output to identify dominant frequencies.",
            "How do I store and version large simulation datasets — HDF5 vs Zarr for array data?",
            "Set up a Jupyter notebook workflow that's reproducible and shareable with collaborators.",
            "How do I visualise high-dimensional simulation results — dimensionality reduction and interactive plots?",
        ],
    },

    // ── Git workflows ─────────────────────────────────────────────────────────
    ConvTemplate {
        seed: "Our team of 12 engineers is struggling with git — long-lived branches, frequent merge conflicts, and broken main. Help us design a better workflow.",
        followups: &[
            "Compare trunk-based development, GitHub flow, and GitFlow — which fits a team shipping daily?",
            "How do I enforce short-lived feature branches — what process changes and tooling help?",
            "Implement a pre-commit hook system with Husky that runs linting and tests before every commit.",
            "How do I handle database migrations in a trunk-based workflow without blocking other developers?",
            "Design the PR review process — size limits, required reviewers, and how to move fast without breaking things.",
            "How do I use git bisect to find which commit introduced a production bug?",
            "Explain git rebase vs merge — when should each be used and what are the golden rules?",
            "How do I handle a production hotfix that needs to go out before the next planned release?",
            "Implement branch protection rules and required status checks to prevent broken code reaching main.",
            "How do I untangle a complicated merge conflict in a file with many interleaved changes?",
            "Design a commit message convention and enforce it with commitlint.",
            "How do I use git worktrees to work on multiple branches simultaneously?",
            "Implement semantic versioning and automated changelogs from conventional commits.",
            "How do I recover from a force push that overwrote someone else's work?",
            "How do I audit the git history to understand when and why a decision was made three years ago?",
        ],
    },

    // ── Mobile development ────────────────────────────────────────────────────
    ConvTemplate {
        seed: "I'm a web developer who needs to build a cross-platform mobile app. Should I use React Native, Flutter, or something else, and how do I get started?",
        followups: &[
            "Compare React Native, Flutter, and Expo in depth — performance, ecosystem, and developer experience.",
            "Walk me through React Native's bridge and the new architecture (JSI/Fabric) — why does it matter?",
            "How do I structure a React Native app — navigation, state management, and API layer?",
            "Implement offline-first data synchronisation with Watermelon DB or MMKV.",
            "How do I handle push notifications on both iOS and Android with proper permission flows?",
            "Implement biometric authentication (Face ID / fingerprint) in React Native.",
            "How do I profile and fix performance issues — JS thread vs main thread and how to diagnose jank?",
            "Design the app's deep linking and universal link strategy for both platforms.",
            "How do I write platform-specific code cleanly without scattering platform checks everywhere?",
            "Implement over-the-air updates with Expo EAS Update — what can and can't be updated without a store release?",
            "How do I set up a CI/CD pipeline that builds and submits to both the App Store and Play Store?",
            "Implement analytics and crash reporting that works correctly across both platforms.",
            "How do I handle different screen sizes, notches, and safe areas across the device landscape?",
            "What does the app store review process look like and what are the common rejection reasons?",
            "How do I implement in-app purchases on both platforms and handle receipt validation securely?",
        ],
    },

    // ── Career in software ────────────────────────────────────────────────────
    ConvTemplate {
        seed: "I've been a software engineer for 3 years and I'm stuck at mid-level. What does it actually take to get promoted to senior engineer and what should I focus on?",
        followups: &[
            "What is the difference in day-to-day work between a mid-level and a senior engineer?",
            "How do I build technical influence without being a manager?",
            "What does 'owning a system' mean in practice — what should I be doing that I'm probably not?",
            "How do I get better at system design — what should I be studying and practising?",
            "How do I make my work more visible without being political or self-promotional?",
            "What does a good engineering design document look like — show me the structure.",
            "How do I mentor junior engineers effectively while still delivering my own work?",
            "How do I handle disagreements with my tech lead or manager about technical direction?",
            "What skills separate good engineers from great ones that are rarely talked about?",
            "How do I evaluate whether I should stay at my company or look for a new role to level up?",
            "How do I prepare for a senior-level technical interview at a top company?",
            "What does an engineering manager actually do — should I consider going into management?",
            "How do I build a reputation in the broader engineering community?",
            "How do I improve my communication skills — writing, presenting, and running meetings?",
            "What does a principal or staff engineer do — is that path right for me?",
        ],
    },

    // ── Cryptography ─────────────────────────────────────────────────────────
    ConvTemplate {
        seed: "I want to deeply understand modern cryptography — not just how to use the libraries, but how the algorithms actually work. Start from the fundamentals.",
        followups: &[
            "Explain modular arithmetic and why it's the foundation of most public-key cryptography.",
            "Walk me through the RSA algorithm — key generation, encryption, decryption, and why it's secure.",
            "What are elliptic curves and why is ECC replacing RSA for most new systems?",
            "Explain the Diffie-Hellman key exchange — how do two parties agree on a secret over a public channel?",
            "How does AES-GCM work — what does authenticated encryption buy me over AES-CBC?",
            "Explain digital signatures — how does ECDSA work and what security properties does it provide?",
            "What is a cryptographic hash function — what makes SHA-256 secure and what would break it?",
            "How does TLS 1.3 work end-to-end — key exchange, cipher suite negotiation, and the handshake?",
            "Explain forward secrecy — why does it matter and how do ephemeral keys provide it?",
            "What is a zero-knowledge proof and where are they used in practice?",
            "How do hardware security modules work and when should I use one?",
            "Explain the Signal protocol — how does it achieve forward secrecy and break-in recovery?",
            "What is post-quantum cryptography and why is it urgent — how does Kyber work?",
            "How do password hashing functions (Argon2, bcrypt) differ from regular hash functions?",
            "Implement a simple Diffie-Hellman exchange in Python from scratch using only modular arithmetic.",
        ],
    },

    // ── Game development ──────────────────────────────────────────────────────
    ConvTemplate {
        seed: "I want to build a 2D multiplayer game in Godot 4. Players explore a procedurally generated world and can fight each other. Walk me through the architecture.",
        followups: &[
            "Design the scene hierarchy in Godot 4 — how should I structure player, world, and UI scenes?",
            "Implement procedural world generation using noise functions — a tile-based approach with biomes.",
            "How do I implement client-server networking in Godot with rollback netcode for a fighting game?",
            "Implement player movement with proper input handling, animation state machine, and collision.",
            "How do I design a combat system — hitboxes, hurtboxes, and lag compensation for melee attacks?",
            "Design the inventory system with drag-and-drop UI and serialisation for save/load.",
            "How do I implement a camera system that follows the player smoothly with screen shake effects?",
            "Implement a simple AI for enemies using behaviour trees — patrol, chase, and attack states.",
            "How do I optimise rendering performance for a large tile-based world — chunking and culling?",
            "Design the game loop for a server-authoritative multiplayer game — where does game state live?",
            "Implement a save system that serialises and deserialises the full world and player state.",
            "How do I add procedural sound effects and music that react to game state?",
            "Implement a modding system so players can add custom items and map tiles.",
            "How do I handle cheating in a multiplayer game — server-side validation and anti-cheat strategies?",
            "Design the deployment: dedicated server infrastructure, matchmaking, and regional scaling.",
        ],
    },
];

/// System prompts sampled independently per conversation.
/// Chosen to elicit verbose, detailed responses that grow conversation length
/// quickly and stress-test the context window.
pub static SYSTEM_PROMPTS: &[&str] = &[
    // Verbose technical expert personas
    "You are a senior software engineer with 20 years of experience. For every question, provide a thorough answer that includes: a conceptual explanation, a complete working code example with comments, common pitfalls to avoid, and production considerations. Never give a short answer when a detailed one is possible.",
    "You are an expert software architect. Always structure your responses with: an executive summary, detailed technical explanation, multiple implementation approaches with trade-offs, a recommended approach with full code, and a section on testing and observability. Use markdown headers and code blocks throughout.",
    "You are a principal engineer at a FAANG company conducting a deep technical review. For every topic, explain it as if writing internal documentation: include background context, the problem being solved, the solution in detail, edge cases, failure modes, and links to related concepts. Be exhaustive.",
    "You are a technical educator writing a textbook chapter. Every response should read like a well-structured chapter: introduction, core concepts with definitions, worked examples with full code, exercises, and a summary. Assume the reader is intelligent but unfamiliar with the topic.",
    "You are a meticulous code reviewer. When shown any code or asked about implementation, always provide: the naive approach first, then progressively more optimised solutions, full working code for each, Big-O analysis, memory usage analysis, and a comparison table of approaches.",

    // Verbose pedagogical styles
    "You are a Socratic tutor. Answer every question by first building intuition from first principles, then introducing formalism, then showing a concrete example, then a more complex example, and finally connecting to the broader landscape. Never skip steps.",
    "You are a systems thinking expert. For every topic, always explain: the components and their interactions, the feedback loops, the failure modes, the edge cases, and how the system behaves under stress. Use diagrams described in ASCII art where helpful.",
    "You are writing a detailed technical blog post in response to every question. Include: a hook, background context, the core explanation with code, real-world examples, benchmarks or measurements where relevant, gotchas, and a conclusion with takeaways.",
    "You are a staff engineer mentoring a junior engineer. Explain everything thoroughly as if they need to understand it deeply enough to debug it in production at 3am. Include war stories, common mistakes, and the reasoning behind every decision.",
    "You are an expert who believes strongly in showing rather than telling. For every explanation, provide at least three complete, runnable code examples that demonstrate different aspects of the concept. Comment every non-obvious line.",

    // Domain-specific verbose personas
    "You are a distributed systems expert. Always frame answers in terms of: consistency models, failure scenarios, network partition behaviour, latency trade-offs, and operational complexity. Include sequence diagrams described in text and discuss CAP theorem implications.",
    "You are a security engineer performing a threat model. For every system or code discussed, identify: the attack surface, potential vulnerabilities, mitigations, defence-in-depth strategies, and monitoring/detection approaches. Be thorough and assume a sophisticated attacker.",
    "You are a performance engineer. For every system or algorithm discussed, always cover: theoretical complexity, practical performance characteristics, profiling methodology, optimisation strategies from low-hanging fruit to advanced, and how to measure the improvement.",
    "You are an SRE writing a runbook. Structure every response as an operational document: overview, prerequisites, step-by-step procedure with expected outputs, troubleshooting section for common failures, rollback procedure, and success criteria.",
    "You are a database administrator with deep expertise. For every question, cover: the data model implications, query patterns and their performance, indexing strategy, transaction semantics, failure handling, backup and recovery, and monitoring.",

    // Extra verbose / comprehensive
    "You are an AI assistant that believes in completeness above all else. Never truncate an answer. If a question has multiple aspects, address all of them. If code is involved, always show the full file, not snippets. If there are trade-offs, enumerate all of them.",
    "You are a technical interviewer at a top company. For every topic, cover it at the depth expected for a principal-level interview: theory, implementation details, optimisations, real-world applications, and follow-up questions with their answers.",
    "You are writing an RFC (Request for Comments) document. Structure every response as a formal RFC: Abstract, Motivation, Detailed Design, Drawbacks, Alternatives Considered, Unresolved Questions, and Implementation Plan.",
    "You are an expert who communicates exclusively through detailed examples. For every concept, provide five progressively complex examples, each with full code, expected output, and an explanation of what it demonstrates. Never explain without showing.",
    "You are a technical writer producing documentation for a complex open source project. Every response should be formatted as official documentation: overview, quick start, detailed reference, configuration options, examples, FAQ, and troubleshooting.",

    // Reasoning-heavy personas (especially good for thinking models)
    "You are a rigorous engineer who thinks step by step. Before giving any answer, explicitly write out your reasoning process: what you know, what you need to figure out, the approach you'll take, and why. Then provide the full solution.",
    "You are an expert who stress-tests every idea. For every proposal or implementation, always ask: what can go wrong? What are the edge cases? What happens under load? What happens when dependencies fail? Provide solutions to each identified problem.",
    "You are a consultant writing a detailed technical proposal. Every response should be structured as a consulting deliverable: Executive Summary, Current State Analysis, Proposed Solution (with multiple options), Implementation Roadmap, Risk Register, and Cost-Benefit Analysis.",
    "You are a compiler for understanding — you take high-level questions and decompose them into their fundamental components, explain each component in depth, show how they compose, and then synthesise back to the original question with a complete answer.",
    "You are an expert debugger. For every technical problem, apply scientific method: form a hypothesis, design experiments to test it, show the test and result, revise the hypothesis, and repeat until the root cause is found. Document every step.",
];

