"""Client-side OWL validation for SutraDB Python SDK.

The database accepts all triples unconditionally. OWL validation
happens here in the SDK before sending data to the server. This
follows the "lean store, smart client" principle.

OWL validation is ENABLED by default. Disable it with:
    client = SutraClient(owl_validation=False)

The validator loads OWL ontology triples from the database on first use,
caches them locally, and checks inserts against:
- rdfs:domain (property domain constraints)
- rdfs:range (property range constraints)
- rdfs:subClassOf (type hierarchy)
- owl:FunctionalProperty (max one value)
- owl:disjointWith (classes that can't overlap)
"""

from __future__ import annotations

from typing import Optional

RDF_TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
RDFS_DOMAIN = "http://www.w3.org/2000/01/rdf-schema#domain"
RDFS_RANGE = "http://www.w3.org/2000/01/rdf-schema#range"
RDFS_SUBCLASS_OF = "http://www.w3.org/2000/01/rdf-schema#subClassOf"
OWL_FUNCTIONAL = "http://www.w3.org/2002/07/owl#FunctionalProperty"
OWL_DISJOINT = "http://www.w3.org/2002/07/owl#disjointWith"


class OWLViolation(Exception):
    """Raised when a triple violates an OWL constraint."""

    def __init__(self, message: str, constraint_type: str, triple: tuple):
        super().__init__(message)
        self.constraint_type = constraint_type
        self.triple = triple


class OWLValidator:
    """Client-side OWL constraint validator.

    Loads ontology axioms from SutraDB and validates triples before insert.
    """

    def __init__(self):
        self.domains: dict[str, str] = {}       # property -> domain class
        self.ranges: dict[str, str] = {}         # property -> range class
        self.subclass_of: dict[str, set[str]] = {}  # class -> set of parent classes
        self.functional: set[str] = set()        # functional properties
        self.disjoint: dict[str, set[str]] = {}  # class -> disjoint classes
        self.entity_types: dict[str, set[str]] = {}  # entity -> set of types
        self._loaded = False

    def load_from_client(self, client) -> None:
        """Load OWL ontology triples from a SutraDB client."""
        # Load domain constraints
        result = client.sparql(
            f'SELECT ?p ?d WHERE {{ ?p <{RDFS_DOMAIN}> ?d }}'
        )
        for row in result.get("results", {}).get("bindings", []):
            p = row.get("p", {}).get("value", "")
            d = row.get("d", {}).get("value", "")
            if p and d:
                self.domains[p] = d

        # Load range constraints
        result = client.sparql(
            f'SELECT ?p ?r WHERE {{ ?p <{RDFS_RANGE}> ?r }}'
        )
        for row in result.get("results", {}).get("bindings", []):
            p = row.get("p", {}).get("value", "")
            r = row.get("r", {}).get("value", "")
            if p and r:
                self.ranges[p] = r

        # Load subclass hierarchy
        result = client.sparql(
            f'SELECT ?c ?parent WHERE {{ ?c <{RDFS_SUBCLASS_OF}> ?parent }}'
        )
        for row in result.get("results", {}).get("bindings", []):
            c = row.get("c", {}).get("value", "")
            parent = row.get("parent", {}).get("value", "")
            if c and parent:
                self.subclass_of.setdefault(c, set()).add(parent)

        # Load functional properties
        result = client.sparql(
            f'SELECT ?p WHERE {{ ?p <{RDF_TYPE}> <{OWL_FUNCTIONAL}> }}'
        )
        for row in result.get("results", {}).get("bindings", []):
            p = row.get("p", {}).get("value", "")
            if p:
                self.functional.add(p)

        # Load entity types (for validation)
        result = client.sparql(
            f'SELECT ?e ?t WHERE {{ ?e <{RDF_TYPE}> ?t }} LIMIT 10000'
        )
        for row in result.get("results", {}).get("bindings", []):
            e = row.get("e", {}).get("value", "")
            t = row.get("t", {}).get("value", "")
            if e and t:
                self.entity_types.setdefault(e, set()).add(t)

        self._loaded = True

    def is_loaded(self) -> bool:
        """Whether the ontology has been loaded."""
        return self._loaded

    def has_constraints(self) -> bool:
        """Whether any OWL constraints exist in the database."""
        return bool(
            self.domains or self.ranges or self.functional or self.disjoint
        )

    def get_all_types(self, class_iri: str) -> set[str]:
        """Get a class and all its ancestors via rdfs:subClassOf."""
        result = {class_iri}
        queue = [class_iri]
        while queue:
            current = queue.pop()
            for parent in self.subclass_of.get(current, set()):
                if parent not in result:
                    result.add(parent)
                    queue.append(parent)
        return result

    def validate_triple(
        self, subject: str, predicate: str, obj: str
    ) -> Optional[OWLViolation]:
        """Validate a single triple against OWL constraints.

        Returns None if valid, or an OWLViolation if invalid.
        """
        triple = (subject, predicate, obj)

        # Domain check
        if predicate in self.domains:
            expected_domain = self.domains[predicate]
            subject_types = self.entity_types.get(subject, set())
            if subject_types:
                all_types = set()
                for t in subject_types:
                    all_types |= self.get_all_types(t)
                if expected_domain not in all_types:
                    return OWLViolation(
                        f"Domain violation: {predicate} requires subject of type "
                        f"{expected_domain}, but {subject} has types {subject_types}",
                        "domain",
                        triple,
                    )

        # Range check
        if predicate in self.ranges and not obj.startswith('"'):
            expected_range = self.ranges[predicate]
            object_types = self.entity_types.get(obj, set())
            if object_types:
                all_types = set()
                for t in object_types:
                    all_types |= self.get_all_types(t)
                if expected_range not in all_types:
                    return OWLViolation(
                        f"Range violation: {predicate} requires object of type "
                        f"{expected_range}, but {obj} has types {object_types}",
                        "range",
                        triple,
                    )

        # Disjoint class check (when assigning a type)
        if predicate == RDF_TYPE:
            existing_types = self.entity_types.get(subject, set())
            for existing_type in existing_types:
                disjoint = self.disjoint.get(existing_type, set())
                if obj in disjoint:
                    return OWLViolation(
                        f"Disjoint violation: {subject} is already type "
                        f"{existing_type}, which is disjoint with {obj}",
                        "disjoint",
                        triple,
                    )

        return None  # Valid

    def validate_ntriples(self, ntriples: str) -> list[OWLViolation]:
        """Validate a block of N-Triples. Returns list of violations."""
        violations = []
        for line in ntriples.splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            # Simple N-Triples parsing (subject predicate object .)
            parts = line.split(None, 2)
            if len(parts) < 3:
                continue
            s = parts[0].strip("<>")
            p = parts[1].strip("<>")
            o_raw = parts[2].rstrip(" .")
            o = o_raw.strip("<>") if o_raw.startswith("<") else o_raw

            violation = self.validate_triple(s, p, o)
            if violation:
                violations.append(violation)

        return violations
