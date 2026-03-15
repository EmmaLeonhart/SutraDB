"""Unit tests for the SutraDB Python client.

These tests verify client construction and method signatures without
requiring a running SutraDB server.
"""

import unittest

from sutradb import SutraClient, __version__
from sutradb.client import SutraError


class TestClientConstruction(unittest.TestCase):
    """Verify that the client can be instantiated with various arguments."""

    def test_default_endpoint(self) -> None:
        client = SutraClient()
        self.assertEqual(client.endpoint, "http://localhost:3030")

    def test_custom_endpoint(self) -> None:
        client = SutraClient("http://db.example.com:9999")
        self.assertEqual(client.endpoint, "http://db.example.com:9999")

    def test_trailing_slash_stripped(self) -> None:
        client = SutraClient("http://localhost:3030/")
        self.assertEqual(client.endpoint, "http://localhost:3030")

    def test_session_created(self) -> None:
        client = SutraClient()
        self.assertIsNotNone(client._session)


class TestMethodSignatures(unittest.TestCase):
    """Verify that all public methods exist with expected signatures."""

    def setUp(self) -> None:
        self.client = SutraClient()

    def test_health_exists(self) -> None:
        self.assertTrue(callable(self.client.health))

    def test_sparql_exists(self) -> None:
        self.assertTrue(callable(self.client.sparql))

    def test_insert_triples_exists(self) -> None:
        self.assertTrue(callable(self.client.insert_triples))

    def test_declare_vector_exists(self) -> None:
        self.assertTrue(callable(self.client.declare_vector))

    def test_insert_vector_exists(self) -> None:
        self.assertTrue(callable(self.client.insert_vector))

    def test_insert_vectors_batch_exists(self) -> None:
        self.assertTrue(callable(self.client.insert_vectors_batch))


class TestSutraError(unittest.TestCase):
    """Verify the custom exception class."""

    def test_basic_error(self) -> None:
        err = SutraError("something went wrong")
        self.assertEqual(str(err), "something went wrong")
        self.assertIsNone(err.status_code)

    def test_error_with_status_code(self) -> None:
        err = SutraError("not found", status_code=404)
        self.assertEqual(err.status_code, 404)

    def test_is_exception(self) -> None:
        self.assertTrue(issubclass(SutraError, Exception))


class TestVersion(unittest.TestCase):
    """Verify the package exposes a version string."""

    def test_version_format(self) -> None:
        parts = __version__.split(".")
        self.assertEqual(len(parts), 3)
        for part in parts:
            self.assertTrue(part.isdigit())


if __name__ == "__main__":
    unittest.main()
