"""LangChain integration for SutraDB.

Provides SutraDB as both a VectorStore and a knowledge graph for
Retrieval-Augmented Generation (RAG) pipelines.

Usage:
    from sutradb.langchain import SutraVectorStore

    vectorstore = SutraVectorStore(
        endpoint="http://localhost:3030",
        predicate="http://sutra.dev/hasEmbedding",
    )

    # Use with LangChain
    retriever = vectorstore.as_retriever()
    docs = retriever.get_relevant_documents("What is a transformer?")

Requires: pip install langchain-core
"""

from __future__ import annotations

from typing import Any, Iterable, Optional

try:
    from langchain_core.documents import Document
    from langchain_core.vectorstores import VectorStore
    from langchain_core.embeddings import Embeddings
except ImportError:
    raise ImportError(
        "langchain-core is required for LangChain integration. "
        "Install it with: pip install langchain-core"
    )

from .client import SutraClient


class SutraVectorStore(VectorStore):
    """LangChain VectorStore backed by SutraDB.

    Uses SutraDB's HNSW vector index for similarity search and
    the RDF triple store for metadata/knowledge graph queries.
    """

    def __init__(
        self,
        endpoint: str = "http://localhost:3030",
        predicate: str = "http://sutra.dev/hasEmbedding",
        embedding: Optional[Embeddings] = None,
        dimensions: int = 1024,
        **kwargs: Any,
    ):
        self._client = SutraClient(endpoint, owl_validation=False)
        self._predicate = predicate
        self._embedding = embedding
        self._dimensions = dimensions

        # Ensure vector predicate is declared
        try:
            self._client.declare_vector(predicate, dimensions)
        except Exception:
            pass  # May already exist

    @property
    def embeddings(self) -> Optional[Embeddings]:
        return self._embedding

    def add_texts(
        self,
        texts: Iterable[str],
        metadatas: Optional[list[dict]] = None,
        **kwargs: Any,
    ) -> list[str]:
        """Add texts with embeddings to SutraDB."""
        if self._embedding is None:
            raise ValueError("Embeddings model required for add_texts")

        texts_list = list(texts)
        vectors = self._embedding.embed_documents(texts_list)
        ids = []

        for i, (text, vector) in enumerate(zip(texts_list, vectors)):
            # Generate a subject IRI
            import hashlib
            text_hash = hashlib.md5(text.encode()).hexdigest()[:12]
            subject = f"http://sutra.dev/doc/{text_hash}"

            # Insert vector
            self._client.insert_vector(self._predicate, subject, vector)

            # Insert text as a triple
            escaped = text.replace('"', '\\"').replace('\n', '\\n')
            ntriples = f'<{subject}> <http://sutra.dev/text> "{escaped}" .'

            # Insert metadata
            if metadatas and i < len(metadatas):
                for key, value in metadatas[i].items():
                    escaped_val = str(value).replace('"', '\\"')
                    ntriples += f'\n<{subject}> <http://sutra.dev/meta/{key}> "{escaped_val}" .'

            self._client.insert_triples(ntriples)
            ids.append(subject)

        return ids

    def similarity_search(
        self,
        query: str,
        k: int = 4,
        **kwargs: Any,
    ) -> list[Document]:
        """Search for similar documents."""
        if self._embedding is None:
            raise ValueError("Embeddings model required for similarity_search")

        query_vector = self._embedding.embed_query(query)
        vec_str = " ".join(f"{v:.6f}" for v in query_vector)

        sparql = (
            f'SELECT ?doc ?text WHERE {{\n'
            f'  VECTOR_SIMILAR(?doc <{self._predicate}> '
            f'"{vec_str}"^^<http://sutra.dev/f32vec>, 0.5, k:={k})\n'
            f'  OPTIONAL {{ ?doc <http://sutra.dev/text> ?text }}\n'
            f'}}'
        )

        result = self._client.sparql(sparql)
        docs = []
        for row in result.get("results", {}).get("bindings", []):
            doc_uri = row.get("doc", {}).get("value", "")
            text = row.get("text", {}).get("value", "")
            docs.append(Document(
                page_content=text or doc_uri,
                metadata={"source": doc_uri},
            ))

        return docs

    @classmethod
    def from_texts(
        cls,
        texts: list[str],
        embedding: Embeddings,
        metadatas: Optional[list[dict]] = None,
        **kwargs: Any,
    ) -> "SutraVectorStore":
        """Create a SutraVectorStore from texts."""
        store = cls(embedding=embedding, **kwargs)
        store.add_texts(texts, metadatas=metadatas)
        return store
