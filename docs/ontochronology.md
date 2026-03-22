# SutraDB — Ontochronology

> Ontochronology: the study and modeling of what exists at which times.
> Time is not metadata on triples. Time is a structural axis of the database.
> Draft v0.1

---

## 1. What Is Ontochronology?

Ontochronology is the formal modeling of entity existence, state, and relationships across time in a queryable knowledge graph. It answers the question: **what was the complete state of the world at time T?**

The term combines *ontology* (what exists) with *chronology* (when it exists). Unlike temporal knowledge graphs that treat time as a qualifier hanging off triples, an ontochronological database treats time as a **first-class axis of the graph topology** — triples are organized along time, not merely annotated with it.

This distinction determines what you can query efficiently:

| Approach | Query | Cost |
|---|---|---|
| **Time as qualifier** | "Was Napoleon emperor at 1810?" | Point lookup on triple, then filter by time qualifier |
| **Time as structural axis** | "What was the complete world state at 1810?" | Single range scan on time-primary index |
| **Time as qualifier** | "What changed between 1804 and 1814?" | Full graph scan, filter every triple's qualifiers |
| **Time as structural axis** | "What changed between 1804 and 1814?" | Two range scans, compute diff |

The second approach makes temporal queries graph-native rather than post-hoc filters. This is the approach SutraDB takes.

---

## 2. Why Ontochronology?

### 2.1 The Gap in Existing Systems

Temporal data exists everywhere. But existing databases handle it poorly:

- **Relational databases** (SQL:2011 bitemporal tables) — temporal queries work within a fixed schema, but "give me the complete state of everything at time T" requires joining across every table. The temporal axis and the entity axis are orthogonal, and querying across both simultaneously is expensive.

- **Temporal knowledge graphs** (Wikidata-style qualifiers) — time is metadata on edges, not structure. The query model is "find triples, then filter by time." This answers "was this true at T?" but not "what was the complete state at T?" efficiently.

- **Time-series databases** (InfluxDB, TimescaleDB) — time is the primary index, everything else is a tag. Fast for "value of X over interval" but useless for relational queries between entities. No concept of "who else was in the room."

- **Event sourcing systems** — model state changes as an append-only log, but reconstruction requires full replay from the beginning unless you maintain snapshots. No native graph traversal.

None of these answer the fundamental ontochronological question: **traverse the graph at a specific moment in time, seeing only what existed then.**

### 2.2 Use Cases

**Text-to-video continuity** — Track every entity (character, prop, location) across every frame or scene. Query: "Who is holding the knife in scene 7? What are they wearing? Were they in the room when the victim entered?" Continuity errors become constraint violations.

**Legal/investigative modeling** — Reconstruct timelines from depositions and evidence. Query: "Where was the defendant between 9am and 11am on March 14th? Which witnesses' statements conflict about that interval?" Chain of custody as a temporal graph trace.

**Historical knowledge bases** — Model entities with imprecise temporal bounds. "George Clayton Abel lived from 1909 to October 29, 1977" — the birth year has year-level precision, the death date has day-level precision. Both are representable without forcing false precision.

**GraphRAG provenance** — Track which extraction pass produced which triples, from which document chunk, at which confidence level. Temporal indexing gives you "what did we know at time T" vs "what do we know now."

**Enterprise state machines** — Loan lifecycles, contract state, approval chains. Query the complete state of a loan at any point in its history. Bitemporal: distinguish "what was actually true" from "what was recorded."

---

## 3. The Ordering Axis

### 3.1 Not Always a Clock

The "T" in TSPO is not necessarily a UTC timestamp. It is an **ordered scalar** — any value that can be sorted and range-scanned. The indexing is identical regardless of what the scalar represents, because the data structure is always a B-tree over ordered keys.

The default is UTC timestamps, because most databases deal with real-world time. But many domains have a natural ordering axis that is not a clock:

| Domain | Ordering Axis | Example Values |
|---|---|---|
| Historical events | UTC timestamps (default) | `"1804-05-18"`, `"2024-03-14T10:00:00"` |
| Screenplays | Scene numbers | `1`, `2`, `3.5` (for inserted scenes) |
| Video production | Frame numbers or timecodes | `0`, `1`, `2`, ..., `86400` |
| Film continuity | Minutes into movie | `0.0`, `12.5`, `90.3` |
| Novels / books | Page numbers or chapter.paragraph | `1`, `42`, `300` |
| Religious texts | Book.chapter.verse | `1.1.1`, `1.1.2`, `66.22.21` |
| Legal proceedings | Exhibit numbers or transcript page | `1`, `2`, `47` |
| Music | Measure numbers or seconds | `1`, `2`, `3`, `4` |
| Software | Version numbers or commit ordinals | `1`, `2`, `3`, `1000` |

The axis type is a **database-wide setting** configured at creation time. Once set, it applies to all temporal predicates in that database. You do not mix frame numbers and UTC timestamps in the same TSPO index — that would make range scans meaningless.

```
# Database creation with non-default axis
sutra create movie.sdb --temporal-axis=integer    # frame/scene numbers
sutra create scripture.sdb --temporal-axis=float   # chapter.verse as float
sutra create events.sdb                            # default: UTC timestamp
```

The implementation cost of supporting different axis types is near zero. The B-tree doesn't care what the bytes represent — it only needs a total ordering. An integer axis, a float axis, and a timestamp axis all produce the same index structure with the same performance characteristics.

### 3.2 Implications for Non-Temporal Axes

When the ordering axis is not a clock, some concepts adapt:

- **"Assertion time"** becomes **"assertion position"** — "known to be the case at this point in the sequence."
- **"Start time" / "end time"** become **"start position" / "end position"** — the interval during which a fact holds.
- **Precision** may not apply (frame 42 is exact; there's no "decade-level precision" for scene numbers).
- **The world state query** becomes "give me the complete state at position P" instead of "at time T" — same range scan, different semantics.

The SPARQL+ operators (`AT_TIME`, `DURING`, `WORLD_STATE`, `TEMPORAL_DIFF`) work identically regardless of axis type. The operator names reference time because that's the common case, but they accept any ordered scalar matching the database's axis type.

---

## 4. Temporal Model (UTC Default)

### 4.1 Three Temporal Signifiers

Every triple in SutraDB can carry up to three temporal signifiers. None are required — some triples are intrinsically atemporal (definitional facts, ontological axioms). A triple can have zero, one, two, or all three.

| Signifier | Meaning | When to use |
|---|---|---|
| **Assertion time** | "This fact was known to be the case at this time" | When start/end times are unknown. The crutch — a proxy for "true as of when we recorded it." |
| **Start time** | "This fact became true at this time" | When the onset of the fact is known. |
| **End time** | "This fact stopped being true at this time" | When the termination of the fact is known. |

These map onto the bitemporal model from database theory:
- **Start time / end time** = valid time (when something was true in the world)
- **Assertion time** = transaction time (when it was recorded)

But the key insight is that **assertion time is a fallback, not a parallel axis.** In a well-instrumented system, most triples have start/end times. Assertion time is the crutch you reach for when the world-state transition happened but nobody was watching.

### 4.2 What Assertion Time Actually Is

Assertion time says: "We have evidence this fact was the case at this moment." It does not assert when it started or ended. It is a **point attestation** — like a photograph. The triple is grounded to a witness, a document, an observation at a specific moment.

For a very large amount of real-world data, start and end times are simply not known. We only know that something was observed to be the case at a particular time. A newspaper article from 1847 tells us a building existed in 1847. We don't know when it was built. We don't know when (or if) it was demolished. Assertion time handles this without forcing us to fabricate interval endpoints.

The inference rule: a fact asserted at time T is likely true indefinitely into the past and future unless contradicted, but its relevance decays with temporal distance from T.

### 4.3 Precision

Temporal signifiers carry a precision level:

| Precision | Example | Meaning |
|---|---|---|
| Millennium | 2000 | "Sometime in this millennium" |
| Century | 1800 | "Sometime in the 19th century" |
| Decade | 1840 | "Sometime in the 1840s" |
| Year | 1847 | "Sometime in 1847" |
| Month | 1847-03 | "Sometime in March 1847" |
| Day | 1847-03-15 | "On March 15, 1847" |
| Hour | 1847-03-15T09 | "During the 9am hour" |
| Minute | 1847-03-15T09:32 | "At 9:32am" |
| Second | 1847-03-15T09:32:00 | "At exactly 9:32:00am" |
| Millisecond | 1847-03-15T09:32:00.000 | Sub-second precision |

Precision is not confidence. A fact with year-level precision is not "less certain" — it's genuinely imprecise. The granularity is part of the truth claim, not an epistemic hedge. Historical facts especially have this property: you know something happened in a decade, not a day.

### 4.4 Open Intervals and Absence

- A triple with a start time and no end time: **open-ended interval** — "still true as far as we know."
- A triple with an end time and no start time: "existed before our record begins, ended here."
- A triple with only assertion time: "we know it was the case at this moment, nothing more."
- A triple with no temporal signifiers at all: **atemporal** — intrinsically and permanently true. "2 + 2 = 4" doesn't have a start time.

The absence of temporal signifiers is not null — it is the correct representation. In the RDF open world, if something is not stated, it is unknown, not false. A triple without a start time doesn't have an "unknown" start time — the start time is simply not asserted.

### 4.5 Multiple Valid Times

A single triple can be valid at multiple disjoint time intervals. A person can hold the same job title at different periods (left, returned). A building can be used as a school, converted to offices, then converted back to a school. Each interval is a separate temporal annotation on the same triple.

This means the temporal annotation is not a single (start, end) pair — it is a **set of intervals** attached to the triple. Implementation-wise, each interval is a separate index entry pointing to the same triple.

---

## 5. Indexing Strategy

### 5.1 Time-Primary Index (TSPO)

The core ontochronological index adds time as a **leading key component**:

```
Standard triple index:  (Subject, Predicate, Object)
Time-primary index:     (Time, Subject, Predicate, Object)
```

This is the same insight applied to any dimension: **promote the query axis to a leading index key.**

With TSPO, "give me the complete world state at time T" is a single range scan on the first key component. Every triple valid at T falls out with no joins, no filter passes, no graph traversal.

### 5.2 Why This Is Cheap

Time indexing is a 1D exact range query on ordered data. That's a B-tree — the simplest, most well-understood index structure in computer science.

Compare to SutraDB's existing indexes:

| Index | Dimensions | Data Structure | Algorithmic Complexity |
|---|---|---|---|
| SPO/POS/OSP | N/A (key permutations) | B-tree / LSM | O(log n) |
| VECTOR(p) | 768–1536 dimensions | HNSW graph | O(log n) approximate |
| **TSPO** | **1 dimension** | **B-tree / LSM** | **O(log n) exact** |

HNSW solves a genuinely hard problem — approximate search in high-dimensional space. Temporal indexing is trivially cheap by comparison. It's just "put time first in the key." The overhead is basically just storage for the additional index.

### 5.3 Coordinate Indexing (Optional)

The same pattern extends to spatial data:

```
Temporal:   (T, S, P, O)       — slice the world at any moment
Spatial:    (X, Y, S, P, O)    — slice the world at any location
Combined:   (T, X, Y, S, P, O) — everything at location L during interval T
```

Coordinate indexing is opt-in. Not every dataset has spatial data, and forcing it as a mandatory dimension would be a design mistake. But when present, spatial queries are just range scans on composite keys — textbook 1980s algorithms.

| Query | Index | Data Structure |
|---|---|---|
| "Everything at time T" | TSPO | B-tree (1D range scan) |
| "Everything at location (X,Y)" | XYSPO | R-tree or composite B-tree (2D range scan) |
| "Everything at location L during interval T" | TXYSPO | Composite range scan |

The dimensional spectrum for SutraDB's indexes:

- **1D** (time, confidence, version): B-tree
- **2–3D** (coordinates): R-tree or composite B-tree
- **4–20D** (low-dimensional embeddings): KD-tree variants
- **20D+** (embeddings): HNSW

SutraDB already pays the hard tax for the high-dimensional end. Low-dimensional indexing is a rounding error on implementation cost.

### 5.4 Provenance as a Low-Dimensional Index

The same pattern generalizes beyond time and space to any low-dimensional metadata axis:

```
Provenance:  (doc_id, chunk_id, pass_id, S, P, O) — where did this triple come from
Confidence:  (score, S, P, O)                     — how reliable is this
Version:     (model_version, S, P, O)              — which extraction model asserted this
```

These compose: `(T, doc_id, confidence, S, P, O)` answers "give me high-confidence triples from source X valid during interval T" as a single range scan.

For GraphRAG, this means traceable reasoning is essentially free. The audit trail of which sources, extraction steps, and confidence levels contributed to an answer becomes a query rather than a pipeline crawl.

---

## 6. World State Queries

### 6.1 The Fundamental Query

The defining query of an ontochronological database:

> **Give me the complete state of the world at time T.**

This returns every triple that was valid at T — every entity that existed, every relationship that held, every attribute that was asserted. It is the temporal equivalent of a full graph dump, but scoped to a moment.

With the TSPO index, this is a range scan: all entries where the time component contains T (either as a point attestation at T, or as an interval containing T).

### 6.2 Temporal Diff

> **What changed between T1 and T2?**

Two range scans (world state at T1, world state at T2), then compute the diff. Triples present at T2 but not T1 are assertions. Triples present at T1 but not T2 are retractions. This is the core operation for changelog reconstruction.

### 6.3 Entity History

> **Give me the complete history of entity E.**

This is a subject-primary query with temporal ordering. The standard SPO index handles the entity lookup; the temporal signifiers on each triple give the ordering. No TSPO scan needed — just read E's triples and sort by time.

### 6.4 Co-Presence

> **Which entities were co-present during interval [T1, T2]?**

Range scan on TSPO for the interval, then group by subject. Any subject appearing in the result was "active" (had at least one valid triple) during the interval. This is the alibi query — "who was where during the relevant window."

### 6.5 Temporal Graph Traversal

> **Starting from entity E at time T, traverse relationship R through the graph, but only follow edges that were valid at T.**

This is a standard graph traversal with a temporal filter: at each hop, check that the edge triple was valid at T before following it. The TSPO index can provide the temporal filter efficiently.

---

## 7. SPARQL+ Temporal Extensions

### 7.1 AT_TIME

Scope a graph pattern to a specific moment:

```sparql
SELECT ?person ?location WHERE {
  AT_TIME("2024-03-14T10:00:00"^^xsd:dateTime) {
    ?person :locatedIn ?location .
    ?person rdf:type :Suspect .
  }
}
```

Semantics: only match triples that were valid at the specified time. Triples with assertion time at or near T are included with lower priority than triples with valid-time intervals containing T.

### 7.2 DURING

Scope a graph pattern to an interval:

```sparql
SELECT ?person ?location WHERE {
  DURING("2024-03-14T09:00:00"^^xsd:dateTime,
         "2024-03-14T11:00:00"^^xsd:dateTime) {
    ?person :locatedIn ?location .
  }
}
```

Returns triples whose valid-time interval overlaps with the specified interval.

### 7.3 WORLD_STATE

Retrieve the complete state snapshot at a given time:

```sparql
SELECT ?s ?p ?o WHERE {
  WORLD_STATE("2024-03-14T10:00:00"^^xsd:dateTime) {
    ?s ?p ?o .
  }
}
```

This is the fundamental ontochronological query expressed in SPARQL+.

### 7.4 TEMPORAL_DIFF

Compute the difference between two world states:

```sparql
SELECT ?change_type ?s ?p ?o WHERE {
  TEMPORAL_DIFF(
    "2024-03-14T09:00:00"^^xsd:dateTime,
    "2024-03-14T11:00:00"^^xsd:dateTime
  ) {
    ?s ?p ?o .
    BIND(sutra:changeType AS ?change_type)
  }
}
```

Returns triples annotated with whether they were added, removed, or modified between the two timestamps.

### 7.5 Combining Temporal and Vector Queries

Ontochronological queries compose with existing SPARQL+ vector operators:

```sparql
SELECT ?doc ?entity WHERE {
  AT_TIME("2024-06-01"^^xsd:dateTime) {
    ?entity rdf:type :Person .
    ?doc :mentions ?entity .
  }
  VECTOR_SIMILAR(?doc :hasEmbedding "..."^^sutra:f32vec, 0.85)
}
```

This finds documents semantically similar to a query vector that mention persons who existed at the specified time. The temporal filter runs on the TSPO index; the vector search runs on the HNSW index; the query planner interleaves them.

---

## 8. RDF-star Representation

### 8.1 Temporal Signifiers as Annotations

Temporal data is stored using RDF-star annotations on triples:

```turtle
# Assertion time only (the crutch)
<< :building_42 :locatedIn :MainStreet >> sutra:assertedAt "1847"^^sutra:temporal .

# Full valid-time interval
<< :napoleon :heldPosition :Emperor >> sutra:validFrom "1804-05-18"^^sutra:temporal ;
                                       sutra:validTo   "1814-04-11"^^sutra:temporal .

# Open-ended interval (still true)
<< :alice :worksAt :Acme >> sutra:validFrom "2023-01-15"^^sutra:temporal .

# Atemporal fact (no temporal signifiers at all)
:water :chemicalFormula "H2O" .
```

### 8.2 The `sutra:temporal` Datatype

A new literal type that encodes both a timestamp and its precision:

```
"1847"^^sutra:temporal           → year precision
"1847-03"^^sutra:temporal        → month precision
"1847-03-15"^^sutra:temporal     → day precision
"1847-03-15T09:32"^^sutra:temporal → minute precision
```

The precision is derived from the format of the literal, not from a separate field. This keeps the data model simple while preserving precision information.

### 8.3 Multiple Valid Intervals

```turtle
# Person held same title in two separate periods
<< :alice :jobTitle :Director >> sutra:validFrom "2018-01-01"^^sutra:temporal ;
                                 sutra:validTo   "2020-06-30"^^sutra:temporal .

<< :alice :jobTitle :Director >> sutra:validFrom "2022-03-01"^^sutra:temporal ;
                                 sutra:validTo   "2024-01-15"^^sutra:temporal .
```

Each interval is a separate RDF-star annotation. The TSPO index has separate entries for each interval, both pointing to the same underlying triple.

---

## 9. Persistence and World State Snapshots

### 9.1 Event Sourcing Model

The natural storage model for ontochronological data is a **changelog** — an ordered sequence of state changes. The world state at any time T is the result of replaying all changes up to T.

This means the TSPO index is fundamentally an index over events, not states. The "state" at time T is computed by:
1. Finding the nearest snapshot before T
2. Replaying all changes between the snapshot and T

### 9.2 Periodic Snapshots

To avoid full replay on every temporal query, SutraDB can maintain periodic world-state snapshots. These are complete copies of all valid triples at meaningful boundaries:

- **Automatic**: at configurable intervals (hourly, daily, on significant change volume)
- **User-directed**: at domain-meaningful boundaries (scene breaks, chapter ends, transaction batches)

The query model becomes:
- "World state at T" → nearest snapshot + delta replay (fast)
- "What changed between T1 and T2" → diff two snapshots or scan changelog between them
- "History of entity E" → SPO scan, temporally ordered

### 9.3 Persistence-First Inference

For text extraction and narrative modeling, the **default is persistence**: any asserted state propagates forward in time until contradicted.

If a character is described as wearing a red coat in chapter 1, that coat persists indefinitely. The changelog only records changes, not continuations. This means:

1. **Default persistence rule**: asserted state propagates forward until contradiction
2. **Convention-based termination**: domain-specific implicit rules that suspend states (sleeping, swimming, scene changes)
3. **Explicit termination**: direct contradictions in the source text override everything

The conventions are stored as ontology triples in the database itself — they're domain-configurable, not hardcoded.

---

## 10. Relationship to Existing SutraDB Architecture

### 10.1 New Index Type

TSPO joins SPO/POS/OSP/VECTOR as a fifth index type. Like VECTOR indexes, it is opt-in — enabled when temporal predicates (`sutra:assertedAt`, `sutra:validFrom`, `sutra:validTo`) are present in the data.

The query planner treats TSPO the same way it treats all other indexes: as an access path with cost estimates. Temporal queries that benefit from TSPO get routed there; queries that don't simply ignore it.

### 10.2 New Predicates

| Predicate | Domain | Range | Purpose |
|---|---|---|---|
| `sutra:assertedAt` | Quoted triple | `sutra:temporal` | Point attestation time |
| `sutra:validFrom` | Quoted triple | `sutra:temporal` | Interval start time |
| `sutra:validTo` | Quoted triple | `sutra:temporal` | Interval end time |

These are reserved predicates that trigger TSPO indexing when used.

### 10.3 New Literal Type

`sutra:temporal` — a timestamp with embedded precision. Stored internally as a (i64 timestamp, u8 precision) pair for efficient comparison and range scanning.

### 10.4 Compatibility

Ontochronological features are purely additive:
- Existing SPO/POS/OSP indexes are unaffected
- Existing HNSW vector indexes are unaffected
- Existing SPARQL+ queries work unchanged
- Databases without temporal data pay zero cost
- The TSPO index is only built when temporal predicates are present

---

## 11. Design Decisions

### 11.1 Why Not a Separate Temporal Database?

For the same reason SutraDB doesn't use a separate vector database. The whole point is that temporal queries compose with graph traversal and vector search in a single query. Splitting temporal data into a separate system creates the same JSON-handoff problem that SutraDB already solved for vectors.

### 11.2 Why RDF-star Annotations, Not Named Graphs?

Named graphs (the traditional RDF approach to context/provenance) are heavyweight and don't compose well. RDF-star annotations let you attach temporal metadata directly to the triple being annotated, which is the natural structure — "this relationship was valid from X to Y" is a statement about the relationship.

### 11.3 Why Precision, Not Confidence?

Temporal precision and temporal confidence are different things. Precision says "this timestamp has year-level granularity" — it's a fact about the data. Confidence says "we're 80% sure this timestamp is correct" — it's a fact about our belief. Precision is stored on the temporal literal itself. Confidence, if needed, goes on the triple as a separate predicate — it's not specific to temporal data.

### 11.4 Why Assertion Time Is a Crutch

In a perfect world, every fact would have known start and end times. Assertion time exists because the world is imperfect — for most historical data, most extracted data, and most real-time observations, we don't know when a state began or ended. We only know it was observed at a certain time.

As data quality improves, assertion time becomes less necessary. A well-instrumented system should produce mostly start/end time intervals, with assertion time as a fallback for the genuinely unknown.

---

## 12. Implementation Priority

1. **`sutra:temporal` literal type** — timestamp + precision, stored as (i64, u8)
2. **Reserved temporal predicates** — `sutra:assertedAt`, `sutra:validFrom`, `sutra:validTo`
3. **TSPO index** — B-tree with time as leading key, built when temporal predicates are detected
4. **AT_TIME / DURING** — SPARQL+ temporal scope operators
5. **WORLD_STATE** — complete state snapshot query
6. **TEMPORAL_DIFF** — world state comparison
7. **Periodic snapshots** — configurable snapshot boundaries
8. **Coordinate indexing** — XYSPO index (optional, same pattern)
9. **Convention ontology** — persistence rules for text extraction
