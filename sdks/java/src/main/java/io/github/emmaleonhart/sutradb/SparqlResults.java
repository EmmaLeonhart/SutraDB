package io.github.emmaleonhart.sutradb;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Parsed SPARQL JSON results.
 *
 * <p>Wraps the standard SPARQL Query Results JSON Format and provides
 * convenience accessors for variable names and binding rows.</p>
 */
public class SparqlResults {

    private final JSONObject raw;
    private final List<String> variables;
    private final List<Map<String, BindingValue>> bindings;

    /**
     * Parse SPARQL results from a raw JSON object.
     *
     * @param json the full SPARQL results JSON response
     */
    public SparqlResults(JSONObject json) {
        this.raw = json;

        // Parse variable names from head.vars
        JSONArray vars = json.getJSONObject("head").getJSONArray("vars");
        List<String> varList = new ArrayList<>(vars.length());
        for (int i = 0; i < vars.length(); i++) {
            varList.add(vars.getString(i));
        }
        this.variables = Collections.unmodifiableList(varList);

        // Parse bindings
        JSONArray bindingsArr = json.getJSONObject("results").getJSONArray("bindings");
        List<Map<String, BindingValue>> rows = new ArrayList<>(bindingsArr.length());
        for (int i = 0; i < bindingsArr.length(); i++) {
            JSONObject row = bindingsArr.getJSONObject(i);
            Map<String, BindingValue> map = new HashMap<>();
            for (String var : this.variables) {
                if (row.has(var)) {
                    JSONObject val = row.getJSONObject(var);
                    map.put(var, new BindingValue(
                            val.getString("type"),
                            val.getString("value"),
                            val.optString("datatype", null),
                            val.optString("xml:lang", null)
                    ));
                }
            }
            rows.add(Collections.unmodifiableMap(map));
        }
        this.bindings = Collections.unmodifiableList(rows);
    }

    /**
     * Return the variable names in the result set.
     *
     * @return unmodifiable list of variable names
     */
    public List<String> getVariables() {
        return variables;
    }

    /**
     * Return all binding rows.
     *
     * @return unmodifiable list of binding maps
     */
    public List<Map<String, BindingValue>> getBindings() {
        return bindings;
    }

    /**
     * Return the number of result rows.
     *
     * @return row count
     */
    public int size() {
        return bindings.size();
    }

    /**
     * Return the raw JSON response.
     *
     * @return the original JSONObject
     */
    public JSONObject getRaw() {
        return raw;
    }

    /**
     * A single binding value within a SPARQL result row.
     */
    public static class BindingValue {
        private final String type;
        private final String value;
        private final String datatype;
        private final String lang;

        /**
         * Create a new binding value.
         *
         * @param type     RDF term type: "uri", "literal", or "bnode"
         * @param value    lexical value
         * @param datatype datatype IRI, or null
         * @param lang     language tag, or null
         */
        public BindingValue(String type, String value, String datatype, String lang) {
            this.type = type;
            this.value = value;
            this.datatype = datatype;
            this.lang = lang;
        }

        /** Return the RDF term type. */
        public String getType() { return type; }

        /** Return the lexical value. */
        public String getValue() { return value; }

        /** Return the datatype IRI, or null if not a typed literal. */
        public String getDatatype() { return datatype; }

        /** Return the language tag, or null if not a language-tagged literal. */
        public String getLang() { return lang; }

        @Override
        public String toString() {
            StringBuilder sb = new StringBuilder(value);
            if (lang != null) sb.append("@").append(lang);
            if (datatype != null) sb.append("^^").append(datatype);
            return sb.toString();
        }
    }
}
