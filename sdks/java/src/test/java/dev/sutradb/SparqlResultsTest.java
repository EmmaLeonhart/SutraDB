package dev.sutradb;

import org.json.JSONObject;
import org.junit.jupiter.api.Test;

import java.util.List;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Unit tests for {@link SparqlResults} parsing logic.
 */
class SparqlResultsTest {

    @Test
    void parsesStandardSparqlResultsJson() {
        String json = "{" +
                "\"head\":{\"vars\":[\"s\",\"p\",\"o\"]}," +
                "\"results\":{\"bindings\":[" +
                "{\"s\":{\"type\":\"uri\",\"value\":\"http://ex.org/a\"}," +
                " \"p\":{\"type\":\"uri\",\"value\":\"http://ex.org/b\"}," +
                " \"o\":{\"type\":\"literal\",\"value\":\"hello\"}}," +
                "{\"s\":{\"type\":\"uri\",\"value\":\"http://ex.org/c\"}," +
                " \"p\":{\"type\":\"uri\",\"value\":\"http://ex.org/d\"}," +
                " \"o\":{\"type\":\"literal\",\"value\":\"world\",\"xml:lang\":\"en\"}}" +
                "]}}";

        SparqlResults results = new SparqlResults(new JSONObject(json));

        assertEquals(3, results.getVariables().size());
        assertEquals(2, results.size());
        assertEquals(2, results.getBindings().size());
    }

    @Test
    void getVariablesReturnsCorrectNames() {
        String json = "{\"head\":{\"vars\":[\"x\",\"y\"]},\"results\":{\"bindings\":[]}}";
        SparqlResults results = new SparqlResults(new JSONObject(json));

        List<String> vars = results.getVariables();
        assertEquals(2, vars.size());
        assertEquals("x", vars.get(0));
        assertEquals("y", vars.get(1));
    }

    @Test
    void getBindingsReturnsCorrectMaps() {
        String json = "{" +
                "\"head\":{\"vars\":[\"name\"]}," +
                "\"results\":{\"bindings\":[" +
                "{\"name\":{\"type\":\"literal\",\"value\":\"Alice\"}}," +
                "{\"name\":{\"type\":\"literal\",\"value\":\"Bob\"}}" +
                "]}}";

        SparqlResults results = new SparqlResults(new JSONObject(json));
        List<Map<String, SparqlResults.BindingValue>> bindings = results.getBindings();

        assertEquals(2, bindings.size());
        assertEquals("Alice", bindings.get(0).get("name").getValue());
        assertEquals("Bob", bindings.get(1).get("name").getValue());
    }

    @Test
    void bindingValueFieldsAreParsedCorrectly() {
        String json = "{" +
                "\"head\":{\"vars\":[\"v\"]}," +
                "\"results\":{\"bindings\":[{" +
                "\"v\":{\"type\":\"literal\",\"value\":\"42\",\"datatype\":\"http://www.w3.org/2001/XMLSchema#integer\"}" +
                "}]}}";

        SparqlResults results = new SparqlResults(new JSONObject(json));
        SparqlResults.BindingValue val = results.getBindings().get(0).get("v");

        assertEquals("literal", val.getType());
        assertEquals("42", val.getValue());
        assertEquals("http://www.w3.org/2001/XMLSchema#integer", val.getDatatype());
        assertNull(val.getLang());
    }

    @Test
    void bindingValueWithLanguageTag() {
        String json = "{" +
                "\"head\":{\"vars\":[\"label\"]}," +
                "\"results\":{\"bindings\":[{" +
                "\"label\":{\"type\":\"literal\",\"value\":\"Katze\",\"xml:lang\":\"de\"}" +
                "}]}}";

        SparqlResults results = new SparqlResults(new JSONObject(json));
        SparqlResults.BindingValue val = results.getBindings().get(0).get("label");

        assertEquals("literal", val.getType());
        assertEquals("Katze", val.getValue());
        assertNull(val.getDatatype());
        assertEquals("de", val.getLang());
    }

    @Test
    void bindingValueUriHasNullDatatypeAndLang() {
        String json = "{" +
                "\"head\":{\"vars\":[\"s\"]}," +
                "\"results\":{\"bindings\":[{" +
                "\"s\":{\"type\":\"uri\",\"value\":\"http://ex.org/thing\"}" +
                "}]}}";

        SparqlResults results = new SparqlResults(new JSONObject(json));
        SparqlResults.BindingValue val = results.getBindings().get(0).get("s");

        assertEquals("uri", val.getType());
        assertEquals("http://ex.org/thing", val.getValue());
        assertNull(val.getDatatype());
        assertNull(val.getLang());
    }

    @Test
    void emptyResultsAreParsedCorrectly() {
        String json = "{\"head\":{\"vars\":[\"a\",\"b\"]},\"results\":{\"bindings\":[]}}";
        SparqlResults results = new SparqlResults(new JSONObject(json));

        assertEquals(2, results.getVariables().size());
        assertEquals(0, results.size());
        assertTrue(results.getBindings().isEmpty());
    }

    @Test
    void missingVariableInBindingRowIsOmitted() {
        // "b" is declared in vars but not present in the first binding row
        String json = "{" +
                "\"head\":{\"vars\":[\"a\",\"b\"]}," +
                "\"results\":{\"bindings\":[{" +
                "\"a\":{\"type\":\"uri\",\"value\":\"http://ex.org/1\"}" +
                "}]}}";

        SparqlResults results = new SparqlResults(new JSONObject(json));
        Map<String, SparqlResults.BindingValue> row = results.getBindings().get(0);

        assertTrue(row.containsKey("a"));
        assertFalse(row.containsKey("b"));
    }

    @Test
    void getRawReturnsOriginalJson() {
        String jsonStr = "{\"head\":{\"vars\":[\"x\"]},\"results\":{\"bindings\":[]}}";
        JSONObject json = new JSONObject(jsonStr);
        SparqlResults results = new SparqlResults(json);

        assertSame(json, results.getRaw());
    }

    @Test
    void bindingValueToStringFormatsCorrectly() {
        SparqlResults.BindingValue uri = new SparqlResults.BindingValue("uri", "http://ex.org/a", null, null);
        assertEquals("http://ex.org/a", uri.toString());

        SparqlResults.BindingValue langLiteral = new SparqlResults.BindingValue("literal", "hello", null, "en");
        assertEquals("hello@en", langLiteral.toString());

        SparqlResults.BindingValue typedLiteral = new SparqlResults.BindingValue(
                "literal", "42", "http://www.w3.org/2001/XMLSchema#integer", null);
        assertEquals("42^^http://www.w3.org/2001/XMLSchema#integer", typedLiteral.toString());
    }
}
