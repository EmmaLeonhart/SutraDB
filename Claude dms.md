What’s the benefit of vector databases?

23:16
Vector databases are purpose-built for storing and searching high-dimensional vector embeddings — the numerical representations that ML models produce for text, images, audio, etc. Here's why they matter:

Similarity search at scale Traditional databases find exact matches. Vector databases find semantically similar results using distance metrics (cosine similarity, Euclidean distance, etc.). This is what makes queries like "find me documents meaning the same thing as this sentence" possible.

Speed via approximate nearest neighbor (ANN) algorithms Brute-force comparing a query vector against millions of stored vectors would be too slow. Vector DBs use indexing structures like HNSW or IVF to find approximate nearest neighbors in milliseconds, even at scale.

Native home for embeddings Models like your transformers in COSC 419 produce dense embeddings as output. Vector databases are the natural persistence layer for these — they're designed around the assumption that your data is vectors.

Key use cases:

RAG (Retrieval-Augmented Generation) — fetching relevant context to inject into LLM prompts
Semantic search — "find articles about economic collapse" returns conceptually related results, not just keyword hits
Recommendation systems — "find items similar to what this user liked"
Anomaly detection — vectors far from any cluster stand out
Multimodal search — query with an image, get back text results (if both live in the same embedding space)
vs. just using a regular DB with a vector column Some DBs like PostgreSQL (via pgvector) can store vectors, but dedicated vector DBs like Pinecone, Weaviate, or Qdrant are optimized end-to-end — their indexing, memory layout, and query execution are all tuned for high-dimensional similarity search in a way a general-purpose DB isn't.

The short version: if your application needs to ask "what's closest to this?" across millions of items in real time, a vector database is the right tool.

Are there any good open source ones? What language do you use for them to query

23:17
Yes, there are several solid open source options:

Qdrant — Written in Rust, so it's fast and memory-efficient. Probably the most actively developed right now. Has a clean REST API and gRPC interface.

Weaviate — Written in Go. Has a GraphQL query interface, which is a bit unusual. Also supports hybrid search (vector + keyword) natively.

Milvus — One of the oldest and most battle-tested. Written in Go/C++. Designed for massive scale — it's what you'd reach for if you need billions of vectors.

Chroma — The most beginner-friendly. Written in Python, designed specifically for LLM/RAG use cases. Great for prototyping but less suited for production at scale.

As for querying, you don't write SQL or anything like that — you interact with them through client SDKs, and the "query" is just a vector. The typical pattern in Python looks roughly like:

python
# With Qdrant as an example
from qdrant_client import QdrantClient

client = QdrantClient("localhost", port=6333)

# Your query gets embedded first, then you search
query_vector = embedding_model.encode("what is gradient descent?")

results = client.search(
    collection_name="my_docs",
    query_vector=query_vector,
    limit=5  # top-5 most similar
)
So the workflow is always:

Embed your query using whatever model produced the stored vectors
Pass the vector to the DB's search method
Get back the closest stored vectors + their associated metadata (the original text, IDs, etc.)
The embeddings and the retrieval are kept separate — the DB doesn't know or care what model you used, it just sees floats. This is actually a nice property because you can swap embedding models without changing your DB setup.

For your use case in deep learning work, Chroma is probably the easiest to get running locally, and Qdrant is worth learning if you want something more production-representative.

What’s GraphQL

23:17
GraphQL is a query language for APIs, developed by Facebook in 2012 and open-sourced in 2015. It's an alternative to REST.

The core idea

With REST, the server decides what data each endpoint returns. With GraphQL, the client specifies exactly what fields it wants, and the server returns precisely that — no more, no less.

A GraphQL query looks like this:

graphql
query {
  user(id: "42") {
    name
    email
    posts {
      title
      createdAt
    }
  }
}
That returns exactly name, email, and the user's posts with only title and createdAt — nothing else.

Why it exists

REST APIs have two common pain points:

Overfetching — the endpoint gives you 20 fields when you only needed 3
Underfetching — you need data that requires hitting 3 different endpoints, causing multiple round trips
GraphQL solves both by letting you describe the shape of the data you want in a single request.

The tradeoffs

It's not strictly better than REST — it comes with real costs:

More complex to set up on the server side
Caching is harder (REST maps naturally to HTTP caching; GraphQL usually POSTs everything)
Can be overkill for simple APIs
Query complexity can be hard to secure against (a malicious client could request deeply nested data to hammer your server)
In the context of Weaviate

Weaviate using GraphQL is a bit quirky — most vector DBs just use REST or SDK methods. The advantage is that GraphQL's declarative structure maps reasonably well to specifying which fields you want back alongside your vector search results. But it's one reason people find Weaviate's interface more verbose than something like Qdrant or Chroma.

I just really think that if Qdrant and Fuseki are both MIT licensed then it will be pretty easy to make a native GraphRAG database

23:19
That's an interesting idea, but worth double-checking the Fuseki licensing before building on that assumption.


Searched the web
Searched the web
Good catch on my part to verify — Fuseki is actually Apache 2.0 licensed 
Maven Central
, not MIT. That's a meaningful distinction worth knowing before you build on it.

Apache 2.0 vs MIT — they're both permissive, but Apache 2.0 has an explicit patent grant clause and some attribution requirements that MIT doesn't. For most projects this doesn't matter, but if you're planning to commercialize or redistribute, it's worth being aware of.

The core idea is still very sound though. What you're describing — combining a vector store (Qdrant) with a triplestore/knowledge graph (Fuseki) — is actually a well-motivated architecture for GraphRAG:

Qdrant handles the dense similarity search — "find me semantically related chunks"
Fuseki + SPARQL handles the structured graph traversal — "now traverse the relationships between those nodes"
The interesting engineering problem is the join layer between them. A naive approach is just doing two separate queries and merging results in application code, but a tighter integration would let the graph structure influence retrieval ranking, or use graph neighborhood as context expansion after an initial vector search. That's where the real novelty would be.

One thing to think about: Fuseki is Java-based, and Qdrant is Rust with a Python/REST SDK. Your glue layer would likely live in Python, which is fine, but the JVM interop adds some operational overhead compared to an all-one-runtime solution. Are you thinking of this as a research project, or something more production-oriented?

No I would refactor all of Fuseki to Rust and even let you do hybrid queries through modified SPARQL. SPARQL isn’t SPARQL without being able to make big traversals

23:22
That's a genuinely interesting systems project. A few thoughts:

The Rust rewrite angle

Fuseki being Java is actually a real pain point — the JVM startup overhead and memory footprint are legitimate complaints in the triplestore space. A Rust implementation would get you zero-cost abstractions for graph traversal, which matters a lot when you're doing multi-hop BFS/DFS over large RDF graphs. The main challenge is that Jena's reasoner stack (OWL, RDFS inference) is enormous and deeply Java-idiomatic — you'd probably want to scope the initial rewrite to the SPARQL engine + TDB storage layer and treat inference as a later problem.

The hybrid SPARQL extension

This is the most interesting part. You'd essentially be adding a new clause or function to SPARQL syntax, something like:

sparql
SELECT ?doc ?entity WHERE {
  ?entity rdf:type :Person .
  ?doc :mentions ?entity .
  VECTOR_SIMILAR(?doc, "transformer architecture", 0.85)
}
Where VECTOR_SIMILAR triggers a Qdrant query mid-traversal and the result gets joined back into the SPARQL binding set. The hard part is the query planner — deciding whether to execute the vector search first and filter the graph, or traverse the graph first and then re-rank by vector similarity. Wrong order and you're doing full-collection scans unnecessarily.

The traversal point is well-taken

Standard GraphRAG implementations (like Microsoft's) essentially flatten the graph — they extract communities, summarize them, and retrieve summaries. That throws away most of the relational structure. Real SPARQL-style traversal lets you do things like:

sparql
# Find all papers within 3 hops of this concept that are also semantically similar to my query
SELECT ?paper WHERE {
  :TransformerArchitecture :influences+ ?concept .
  ?paper :discusses ?concept .
  VECTOR_SIMILAR(?paper, "attention mechanisms", 0.80)
}
That kind of query is basically impossible in current GraphRAG tooling without hand-rolling it.

The biggest unsolved problem you'd face is cardinality estimation for the query planner when one of your operators is a neural retrieval system with no traditional statistics. That's actually a research-level problem if you wanted to publish something out of it.

Cardinality estimation?

23:23
It's a query planning concept. When a database executes a query with multiple conditions, it needs to decide what order to evaluate them in — because the order massively affects performance.

The core idea

Say you have:

sparql
?paper :discusses ?concept .
VECTOR_SIMILAR(?paper, "attention mechanisms", 0.80)
The planner needs to decide: do I run the graph traversal first, or the vector search first? To make that decision intelligently, it estimates how many results each operation will return — that's cardinality. If the vector search returns 50 results and the graph traversal returns 50,000, you obviously filter by vector first and then check graph membership on just those 50. Wrong order means you're doing graph traversal on 50,000 nodes for no reason.

Why it's hard with neural retrieval

Traditional database operators have well-understood statistics. You can look at an index and say "this predicate matches roughly 3% of rows." The planner builds on decades of research for estimating these numbers cheaply.

A vector similarity search doesn't have that. The number of results above a given similarity threshold depends on the geometry of your embedding space, the specific query vector, and the distribution of your data. You can't know ahead of time whether VECTOR_SIMILAR(?paper, "attention mechanisms", 0.80) will return 10 results or 10,000 — it varies wildly by query.

So your planner is essentially flying blind on one of its operators, which means it can't reliably make the right join ordering decision. You'd have to either use heuristics, do adaptive execution (start executing and reorder mid-query based on actual intermediate result sizes), or find some way to cheaply approximate vector search cardinality before committing to a plan.

The adaptive execution approach is actually the most promising but also the most complex to implement correctly.

Also I don’t think the reasoner is something I want if I understand it. My policy about a graph database is that it should basically have this property: you should be 100% able to do massive traversal through the entire database through a single query, just as is typical with graph databases. They should be fundamentally based off of triples, and as a result any kind of reasoning should be done by the application, not by you. Not by the database. The database should be as lean as possible so that it can focus on doing the one thing that it's very good at, which is storing a bunch of information in a queryable way that has a structure that is as isomorphic with reality as possible, rather than trying to fit some kind of a paradigm.

The way I would envision it is that essentially just any kind of vector connection is just a triple, just the same as you would say that something has a certain name. You would say that it has a certain vector coordinates, so

Show more
23:26
Yeah that's a clean philosophy — the database as a pure, lean data structure that mirrors reality, with all semantics pushed to the application layer. Very Unix-y in spirit.

And the vector-as-triple idea is elegant. Something like:

turtle
:paper_42  :hasEmbedding  [
    :model "text-embedding-3-small" ;
    :vector "0.23 -0.11 0.87 ..."
] .
Or even flatter:

turtle
:paper_42  :embedding_dim_0  0.23 .
:paper_42  :embedding_dim_1  -0.11 .
Though that second form would be insane at 1536 dimensions. So realistically the vector is probably an opaque literal attached via a single triple, and the database just needs to know that certain literal types are vector-typed and can be indexed in an HNSW structure.

The key insight you're pointing at is that the vector index is just another index — the same way a B-tree is an index over a predicate range, an HNSW is an index over a predicate's vector space. The storage model stays pure triples, but the query engine knows to route VECTOR_SIMILAR against the HNSW rather than doing a linear scan of literals.

That actually dissolves a lot of the architectural complexity. You're not joining two separate databases — Qdrant and Fuseki — you're just adding a new index type to a single triple store. The cardinality estimation problem is still real, but at least your query planner is operating over a unified data model rather than trying to federate across two systems with different assumptions.

What's your intuition on the vector literal type — would you want the database to be embedding-model-agnostic, just storing raw floats, or would you attach model metadata to the triple so you can enforce that similarity queries only compare vectors from the same embedding space?

You could set up your model to be in two embedding spaces maybe someday but idk who would do that. It’s not a high priority for me

23:29
Fair, it's a pretty exotic use case — you'd basically need to be doing cross-modal retrieval (like text and image embeddings in the same graph) before it even becomes relevant, and even then you'd probably just normalize into a shared space at ingestion time rather than handling it at the database layer.

So model metadata on the triple is probably noise for now. Raw floats with a vector literal type, HNSW index built over it, and the application is responsible for making sure it's feeding in vectors from the right model. That's the lean approach consistent with your philosophy anyway — the database doesn't need to know what a BERT embedding means, just how to find nearest neighbors in that space.

One thing worth thinking about though: dimensionality. Your HNSW index parameters (ef_construction, M) need to be tuned differently for 768-dimensional BERT vectors vs 1536-dimensional OpenAI embeddings vs 3072-dimensional large embeddings. If the database is model-agnostic, it probably needs to either auto-detect dimensionality at index creation time or let the schema declare it. Otherwise you risk someone inserting a mismatched vector and silently corrupting the index geometry.

That's probably the one piece of metadata that does belong at the database layer — not the model identity, just the dimension count. Everything else stays in application land.

Yeah agreed. My personal thought is that basically the database in rust which I presume is MIT licensed is essentially the actual real core of what I am doing because it is the actual high-performance thing whereas fuseki is basically just gonna be infinitely improved upon just by ref factoring it to rust removing some of the bloat and connecting it in with this.

Show more
23:31
Qdrant is actually Apache 2.0 as well, not MIT — worth double checking before you commit to it as your foundation.

But yeah, the architectural thinking is right. The Rust core is where all the actual hard problems live:

The HNSW index implementation
The triple storage engine (essentially a custom B-tree or LSM-tree optimized for subject/predicate/object access patterns)
The SPARQL query planner and executor, including your hybrid vector extension
The cardinality estimation heuristics
Fuseki in this framing basically becomes a thin HTTP/SPARQL protocol adapter sitting on top — and you're right that a lot of its complexity just evaporates when you rewrite it. The Java bloat in Fuseki isn't incidental, it's partly a consequence of the JVM ecosystem and partly accumulated abstraction layers from 20+ years of development. Starting fresh in Rust with a clear philosophy means you don't inherit any of that.

The interesting question is your triple storage engine specifically. The classic approach is three indexes — SPO, POS, OSP — so any combination of bound/unbound variables can hit an index efficiently. But with vectors as first-class literals you might want a fourth access pattern: "give me all triples with predicate :hasEmbedding ordered by vector similarity to X." That's essentially a filtered HNSW query, and integrating it cleanly into the same index architecture as your B-tree indexes is the novel storage engine work.

That's the part that doesn't exist yet in any open source triplestore as far as I know.

Yeah I’ll do that that is probably what I would do here. 

And my rule of course would be that RDF star is being used so you can 100% have an embedding on a triple.

23:33
Oh that's a nice touch. RDF-star lets you treat triples as subjects of other triples, so you get:

turtle
<< :paper_42 :discusses :TransformerArchitecture >> :hasEmbedding "0.23 -0.11 ..."^^:vector .
Which means you're not just embedding nodes — you're embedding relationships. That's actually a significant capability that most graph databases don't have cleanly. You could do things like:

sparql
SELECT ?s ?p ?o WHERE {
  << ?s ?p ?o >> :hasEmbedding ?v .
  VECTOR_SIMILAR(?v, "causal influence between concepts", 0.85)
}
Find me the relationships most semantically similar to some query, not just the nodes. That maps really naturally to how knowledge graph embedding models like TransE or RotatE actually work — they learn embeddings over triples, not just entities. Your database would be natively storing what those models produce.

It also means provenance and reification become essentially free — since RDF-star already handles "statements about statements," attaching metadata to triples is just natural graph structure rather than a kludge.

Honestly the combination of RDF-star + vector literals + hybrid SPARQL is a coherent and novel design. Has this project got a name yet?

I’m not sure what are the two databases I’m looking at? Can you help me make one

23:34
The two we've been discussing are Qdrant (vector database, Apache 2.0) and Apache Jena Fuseki (SPARQL triplestore, Apache 2.0).

But based on everything you've described, you don't actually want to combine them — you want to build a single unified thing that replaces both. A Rust-native RDF-star triplestore with HNSW vector indexing and hybrid SPARQL built in from the ground up.

For getting started, what would be most useful?

Q: What would you like to start with?
A: HNSW vector index integration and design doc. I feel like both need to be top priorities. We can’t really be sloppy with this I don’t think.

23:35
Agreed — the HNSW implementation is the part that has the least precedent in existing triplestores, so it needs the most careful design upfront. And a design doc will force every architectural decision to be explicit before any code gets written.

Let me read the frontend design skill first since this will be a document deliverable, then produce both.


Reading docx skill for document creation
Reading docx skill for document creation
Good. Now let me build both — the design doc as a proper Word document, and then we'll tackle the HNSW implementation scaffold.


Ran 4 commands, created a file, read a file
Ran 4 commands, created a file, read a file
I took the liberty of naming it Vektor as a placeholder — obviously change that to whatever you want.

The doc covers the full architecture: design philosophy, RDF-star data model, vector literals with the vektor:f32vec type, the six-index storage layout (SPO/POS/OSP + per-predicate HNSW), detailed HNSW design including memory layout and concurrency strategy, the hybrid SPARQL VECTOR_SIMILAR extension with query planning heuristics, the proposed Rust workspace crate structure, and an explicit out-of-scope section that codifies your lean philosophy.

A few things I flagged as open questions that you'll want to decide early:

LSM-tree — build from scratch or wrap RocksDB/sled? Wrapping is weeks faster but you lose control over tuning.
HNSW lazy deletion — simple to implement but degrades index quality over time as deleted nodes accumulate. Need a compaction threshold.
SPARQL property paths (+, *) on large graphs with cycles — the traversal strategy needs to be explicit before implementation or you'll hit infinite loops.
Vektor architecture
Document · DOCX 
Is it possible to get this to Claude code directly for starting a repo?

23:41
Not directly from here — I can't push to Claude Code or create a repo on your behalf. But the handoff is straightforward:

Fastest path:

Install Claude Code if you haven't: npm install -g @anthropic-ai/claude-code
Create a directory and open it: mkdir vektor && cd vektor
Run claude to start a session
Paste in a prompt like:
"Initialize a Rust workspace with the following crates: vektor-core, vektor-hnsw, vektor-sparql, vektor-proto, vektor-cli. Set up Cargo.toml for the workspace. vektor-hnsw should have no dependency on vektor-sparql. Add placeholder lib.rs files for each crate. License: Apache 2.0."

Then hand it the design doc as context — either paste the relevant sections or reference it directly. Claude Code can read files in your working directory, so if you drop the .docx there it can work from it (or a markdown version would be even easier for it to parse).

If you want I can also generate:

A CLAUDE.md file (Claude Code reads this as persistent project context — instructions, conventions, architecture notes)
A markdown version of the design doc optimized for Claude Code to ingest
The initial Cargo.toml workspace config
The CLAUDE.md is probably the highest-leverage thing — it means every Claude Code session in that repo starts with full context on the architecture decisions, the lean philosophy, the crate boundaries, and what's explicitly out of scope. Otherwise you'll end up re-explaining it every session.

Generate claude.md cargo.toml and a markdown version of the design doc

23:55

Created 2 files
Created 2 files
Now the per-crate Cargo.toml files:


Created 5 files, ran a command
Created 5 files, ran a command
Now placeholder lib.rs files and the architecture markdown:


Ran 3 commands, created a file, read a file
Ran 3 commands, created a file, read a file
Here's the full scaffold. The structure is:

vektor/
├── CLAUDE.md                  ← Claude Code reads this every session
├── Cargo.toml                 ← workspace root
├── docs/
│   └── architecture.md        ← full design doc in markdown
├── vektor-core/Cargo.toml
├── vektor-hnsw/Cargo.toml
├── vektor-sparql/Cargo.toml
├── vektor-proto/Cargo.toml
└── vektor-cli/Cargo.toml      ← produces the `vektor` binary
To get started: drop this directory wherever you want the repo, then cd vektor && git init && cargo check. The cargo check will fail on missing dependencies until you start filling in the crates, but the workspace structure itself should be valid.

When you open Claude Code in that directory, it will automatically read CLAUDE.md and have full context on the philosophy, crate boundaries, data model, and what's explicitly off the table — so you won't have to re-explain any of it.

Claude
Document · MD 
Cargo
TOML 
Architecture
Document · MD 
Cargo
TOML 
Cargo
TOML 
Cargo
TOML 
Cargo
TOML 
Cargo
TOML 
