"""SutraDB Python client."""

from __future__ import annotations

from typing import Any

import requests


class SutraError(Exception):
    """Exception raised by the SutraDB client on request failures."""

    def __init__(self, message: str, status_code: int | None = None) -> None:
        super().__init__(message)
        self.status_code = status_code


class SutraClient:
    """Client for interacting with a SutraDB server.

    Args:
        endpoint: Base URL of the SutraDB HTTP server.
            Defaults to ``http://localhost:3030``.
        owl_validation: Enable client-side OWL constraint validation.
            When True (default), inserts are checked against OWL axioms
            stored in the database before being sent. Raises OWLViolation
            on constraint violations. The database itself always accepts
            all triples regardless of this setting.
    """

    def __init__(
        self,
        endpoint: str = "http://localhost:3030",
        owl_validation: bool = True,
    ) -> None:
        self.endpoint = endpoint.rstrip("/")
        self._session = requests.Session()
        self._session.headers.update({"User-Agent": "sutradb-python/0.1.0"})
        self._owl_validation = owl_validation
        self._owl_validator = None

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    def _url(self, path: str) -> str:
        return f"{self.endpoint}{path}"

    def _request(
        self,
        method: str,
        path: str,
        *,
        params: dict[str, Any] | None = None,
        json: Any | None = None,
        data: str | None = None,
        headers: dict[str, str] | None = None,
    ) -> requests.Response:
        """Send an HTTP request and raise :class:`SutraError` on failure."""
        try:
            resp = self._session.request(
                method,
                self._url(path),
                params=params,
                json=json,
                data=data,
                headers=headers,
            )
        except requests.RequestException as exc:
            raise SutraError(f"Connection error: {exc}") from exc

        if not resp.ok:
            raise SutraError(
                f"HTTP {resp.status_code}: {resp.text}",
                status_code=resp.status_code,
            )
        return resp

    # ------------------------------------------------------------------
    # OWL validation
    # ------------------------------------------------------------------

    def _ensure_owl_loaded(self) -> None:
        """Lazy-load OWL ontology from the database on first validation."""
        if self._owl_validator is not None:
            return
        try:
            from .owl import OWLValidator

            self._owl_validator = OWLValidator()
            self._owl_validator.load_from_client(self)
        except Exception:
            # If we can't load the ontology, skip validation silently
            self._owl_validator = None

    def reload_owl(self) -> None:
        """Force reload of OWL ontology from the database."""
        self._owl_validator = None
        self._ensure_owl_loaded()

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def health(self) -> bool:
        """Check whether the server is reachable.

        Returns:
            ``True`` if the server responds to ``GET /health`` with a 2xx
            status code, ``False`` otherwise.
        """
        try:
            self._request("GET", "/health")
            return True
        except SutraError:
            return False

    def sparql(self, query: str) -> dict:
        """Execute a SPARQL query and return the parsed JSON result.

        Args:
            query: A SPARQL 1.1 query string.

        Returns:
            The JSON response body as a Python dict (SPARQL JSON Results
            format for SELECT/ASK, or a status dict for UPDATE).

        Raises:
            SutraError: If the server returns a non-2xx status code.
        """
        resp = self._request(
            "GET",
            "/sparql",
            params={"query": query},
            headers={"Accept": "application/sparql-results+json"},
        )
        return resp.json()

    def insert_triples(
        self, ntriples: str, batch_size: int = 5000
    ) -> dict[str, Any]:
        """Insert triples in N-Triples format, optionally in batches.

        Args:
            ntriples: One or more triples in N-Triples syntax (one per line).
            batch_size: Maximum number of triples to send per HTTP request.

        Returns:
            A dict ``{"inserted": int, "errors": list[str]}`` summarising the
            outcome across all batches.
        """
        # OWL validation (client-side, before sending to database)
        if self._owl_validation:
            self._ensure_owl_loaded()
            if self._owl_validator and self._owl_validator.has_constraints():
                from .owl import OWLViolation

                violations = self._owl_validator.validate_ntriples(ntriples)
                if violations:
                    raise violations[0]  # Raise first violation

        lines = [
            line for line in ntriples.splitlines() if line.strip()
        ]

        total_inserted = 0
        errors: list[str] = []

        for start in range(0, len(lines), batch_size):
            batch = "\n".join(lines[start : start + batch_size])
            try:
                resp = self._request(
                    "POST",
                    "/triples",
                    data=batch,
                    headers={"Content-Type": "application/n-triples"},
                )
                body = resp.json()
                total_inserted += body.get("inserted", 0)
                batch_errors = body.get("errors", [])
                if batch_errors:
                    errors.extend(batch_errors)
            except SutraError as exc:
                errors.append(str(exc))

        return {"inserted": total_inserted, "errors": errors}

    def declare_vector(
        self,
        predicate: str,
        dimensions: int,
        m: int = 16,
        ef_construction: int = 200,
        metric: str = "cosine",
    ) -> dict:
        """Declare an HNSW-indexed vector predicate.

        Args:
            predicate: The IRI of the vector predicate (e.g.
                ``"http://example.org/hasEmbedding"``).
            dimensions: The fixed dimensionality of vectors for this predicate.
            m: HNSW ``M`` parameter (max connections per node per layer).
            ef_construction: HNSW ``ef_construction`` beam width.
            metric: Distance metric (``"cosine"``, ``"euclidean"``, or
                ``"dot"``).

        Returns:
            The server response as a dict, typically containing ``status`` and
            ``predicate_id`` keys.

        Raises:
            SutraError: If the server rejects the declaration.
        """
        resp = self._request(
            "POST",
            "/vectors/declare",
            json={
                "predicate": predicate,
                "dimensions": dimensions,
                "m": m,
                "ef_construction": ef_construction,
                "metric": metric,
            },
        )
        return resp.json()

    def insert_vector(
        self, predicate: str, subject: str, vector: list[float]
    ) -> dict:
        """Insert a single vector embedding.

        Args:
            predicate: The IRI of the vector predicate.
            subject: The IRI of the subject node.
            vector: The embedding as a list of floats.

        Returns:
            The server response as a dict, typically containing ``status`` and
            ``triple_id`` keys.

        Raises:
            SutraError: If the server rejects the insert.
        """
        resp = self._request(
            "POST",
            "/vectors",
            json={
                "predicate": predicate,
                "subject": subject,
                "vector": vector,
            },
        )
        return resp.json()

    def insert_vectors_batch(
        self,
        predicate: str,
        entries: list[tuple[str, list[float]]],
        batch_size: int = 100,
    ) -> dict[str, Any]:
        """Insert multiple vectors in batches.

        Args:
            predicate: The IRI of the vector predicate.
            entries: A list of ``(subject_iri, vector)`` tuples.
            batch_size: Maximum number of vectors to send per HTTP request.

        Returns:
            A dict ``{"inserted": int, "errors": list[str]}`` summarising
            the outcome across all batches.
        """
        total_inserted = 0
        errors: list[str] = []

        for start in range(0, len(entries), batch_size):
            batch = entries[start : start + batch_size]
            for subject, vector in batch:
                try:
                    result = self.insert_vector(predicate, subject, vector)
                    if result.get("status") == "ok":
                        total_inserted += 1
                except SutraError as exc:
                    errors.append(f"{subject}: {exc}")

        return {"inserted": total_inserted, "errors": errors}
