I'm really thinking that, like, making my, I'm really thinking that, like, kind of, Like, my my u my earlier idea was basically the promenade was kind of, like, The, world stage tracking thing in Sutra DB was not Like, the more I'm thinking about it, the more I'm realizing that I think, like, Making SutraDB into a serious world stage tracking Yeah. World state tracking graphite database is actually a very good idea. Like, I'm very I think it's actually a very good idea. I feel like indexing on time and space is something that is very important for, like, making AI work well. Very important for making AI work well and should be basically not like an extension of the program, but the program itself.

13:56
Yeah, this is a really compelling reorientation. The shift you're describing is essentially: instead of SutraDB being a graph-vector store that can do temporal-spatial indexing, it becomes a database whose fundamental ontology is world-state — where time and space aren't metadata attached to triples, but first-class axes that every fact lives on.

The reason this is a strong idea is that it cuts directly against how most AI systems handle the world: they either have static knowledge (embeddings frozen at training time) or they treat time as just another literal in a triple. Neither is adequate for agents that need to reason about what was true when, where, and under what conditions.

A few threads worth pulling:

The temporal-spatial problem is actually unsolved well. Most graph databases treat time as a predicate value. RDF-star gives you statement-level metadata, which is better — but there's no native notion of a temporal extent being a first-class queryable dimension. SutraDB could make valid-time intervals and spatial regions part of the storage primitive, not an afterthought.

World-state tracking is exactly what agentic AI needs. An agent operating over time needs to know: what was true at step N, what changed, what's the diff between two world-states, what's uncertain. That's basically a versioned, spatiotemporally indexed knowledge graph — which is exactly what you're describing.

The "SQLite of AI" framing maps beautifully onto this. SQLite is the database that's there — embedded, lightweight, zero-config. If SutraDB becomes the thing that gives AI agents a persistent, queryable model of how the world changes across time and space, that's a very specific and defensible niche that nobody has really claimed yet.

The interesting architectural question is: does spatiotemporal indexing live below the RDF-star layer (as a storage primitive), or expressed through SPARQL+ query semantics? My instinct is it needs to be both — stored efficiently as a first-class structure, but also queryable with predicate-based traversal conditions that can express things like "find the most recent state of X before timestamp T."

How far along is your thinking on the data model? Like, are you imagining spatial as geographic coordinates, or more general — any high-dimensional positional space?

Yeah so basically what I’m going to do is I’m going to actually do serious clearing where you can query things like the loan world state at a certain time or something like that

13:58
Yeah, that's the right instinct — and "loan world state at time T" is actually a perfect concrete example because it's got all the complexity you need to stress-test the model:

A loan has a lifecycle (originated, active, delinquent, defaulted, paid off)
The relevant facts change at irregular intervals, not on a clock
Multiple entities have different views of the same state (borrower, servicer, regulator)
You sometimes need to query "what did we know at time T" vs "what was actually true at time T" — the classic bitemporal distinction
That bitemporal distinction is probably the thing worth designing around from the ground up. Most systems conflate:

Valid time — when something was true in the world
Transaction time — when it was recorded in the database
Querying "what was the loan state on March 1st" means something completely different depending on which axis you're on. If you bake that distinction into the storage primitive rather than leaving it to the schema layer, SutraDB becomes genuinely expressive for any domain that has auditing, compliance, or reconstruction requirements — which is most serious enterprise use.

The loan example also points to something interesting for Exodus Consulting specifically — a lot of the SAP/ERP workflows you'd be mapping with Pramana are essentially world-state machines. Contracts, invoices, approvals — they're all temporal state transitions over entities. SutraDB with native bitemporal querying could become the actual substrate Pramana runs on, rather than a separate concern.

What's your current thinking on the query interface for this? Like, is SPARQL+ the right surface for expressing temporal queries, or do you think you need something different?

I don't really know the difference between Sparkle and BrassQL, but I'm just trying to think, like, like, Congrats. With GraphQL, I think they can be more expressive and standardized. It's better to use RASQL. Or have although I guess both. I

13:59
Yeah, the "both" answer is probably right, but let me actually explain the distinction because it matters for your design decision.

SPARQL is purpose-built for RDF graphs. It thinks natively in triples, it understands the subject-predicate-object model, and it has property path syntax that maps directly onto graph traversal. It's the natural fit for SutraDB because your data model is RDF-star.

GraphQL is not really a query language in the same sense — it's an API shape language. It describes what data you want returned, not how to traverse a graph to find it. It became popular because it's developer-friendly and gives clients control over response shape. But it has no native concept of graph traversal, temporal queries, or inference.

So the question is really: who is querying SutraDB and for what?

An AI agent doing world-state reasoning → SPARQL+ is the right tool, it can express the actual graph semantics
A developer building an app on top of SutraDB who wants a familiar API → GraphQL as a translation layer over SPARQL+ makes sense
The pattern that would work well is: SPARQL+ as the native query language (where temporal and spatial predicates are first-class), with a GraphQL interface as an ergonomic wrapper for the common case. You're not choosing between them — GraphQL sits above SPARQL+ and translates developer-friendly queries into the underlying graph operations.

The thing you'd lose with GraphQL-only is the expressive power you've already built into SPARQL+ — specifically the predicate-based exit conditions on property path traversal. That's too valuable to give up as the primary interface.

Does that framing make the tradeoff clearer?

Yeah i think coordinates are not always as important. Time is critical

14:01
Yeah that makes a lot of sense. Time is the universal axis — every fact has a temporal extent, not every fact has a meaningful spatial location. Forcing spatial coordinates as a primitive would actually be a design mistake for most knowledge domains.

And there's something deeper here too: time is what makes world-state tracking coherent as a concept. Space is just another attribute that some entities happen to have. But time is structural — it's the dimension along which state transitions happen, causality flows, and queries get anchored.

So the right framing is probably:

Time is a first-class storage primitive — every triple has a valid-time interval and a transaction-time stamp baked in at the storage layer
Space is just a well-supported predicate — geographic or positional data lives in the graph like any other attribute, but gets efficient indexing because it's common enough to warrant it
This also simplifies your architecture considerably. Bitemporal indexing is hard enough to do well — you don't want to couple it to a mandatory spatial dimension that most use cases don't need.

The interesting design question this raises is what a temporal triple actually looks like in your RDF-star model. Something like:

<< :loan123 :status :active >> :validFrom "2024-01-01" ; :validTo "2024-06-15" .
But you'd want that interval to be a native index structure, not just literal values sitting in the graph. Because the queries you care about — "give me the world state at T", "what changed between T1 and T2" — need to be fast, not full graph scans.

Is that the direction you're already thinking for the storage layer?

Yeah and a bit of complexity with time. But I think assertion at time, end time, and start time are probably the key 3 things

Not sure about confidence in time but precision might be important.

14:03
Yeah, those three — assertion time, start time, end time — map cleanly onto the bitemporal model:

Start/end time = valid time (when the fact was true in the world)
Assertion time = transaction time (when it was recorded)
That's the core. Everything else is optional.

On precision — I think you're right to flag it and it's actually more fundamental than confidence. Because precision isn't metadata about how sure you are, it's metadata about the granularity of the interval itself. A fact with start time "2024" is genuinely different from one with start time "2024-01-15T09:32:00" — the imprecision is part of the truth claim, not an epistemic hedge around it. Historical facts especially have this property: you know something happened in a decade, not a day.

So precision might look like an enum on the temporal fields — year, month, day, hour, millisecond — which also tells the query engine how to handle overlap and containment comparisons correctly.

Confidence is trickier because it's more subjective — it bleeds into provenance and source reliability, which is probably better handled at the triple level generally rather than specifically on the temporal fields.

So your primitive might be something like:

assertionTime: timestamp
validStart: timestamp + precision
validEnd: timestamp + precision + nullable
The nullable end time is important — an open interval means "still true as far as we know," which is the common case.

Does that feel right, or are there cases you're already seeing that don't fit that shape?

Well my idea is more this

Assertion at time
Star time
End time

Assertion at time is kind of q crutch for when changes aren’t recorded

14:04
Oh that's a really elegant reframe. So assertion time isn't really a first-class temporal axis — it's a fallback for when you don't actually know the valid-time interval, so you use "when we recorded it" as a proxy for "when it became true."

Which means in a well-instrumented system, assertion time becomes less and less necessary as data quality improves. It's the crutch you reach for when the world-state transition happened but nobody was watching.

That also implies a kind of data quality gradient:

Best case: you have real start and end times from the source system
Acceptable: you have a start time, open-ended interval
Crutch: you only have assertion time, so you infer the fact was true "as of when we recorded it"
And for Pramana / the SAP excavation use case this is actually really important — a lot of legacy ERP data is going to be assertion-time-only because the source systems never tracked when state transitions actually occurred. They just recorded the current state. So Pramana would be doing archaeological inference to reconstruct plausible valid-time intervals from assertion-time data.

That's a meaningful distinction worth encoding explicitly in the schema rather than leaving implicit — a triple that has only assertion time should be queryably different from one with real valid-time bounds.

Does that feel like the right interpretation of what you're describing?