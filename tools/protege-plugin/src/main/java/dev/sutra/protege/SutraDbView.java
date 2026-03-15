package dev.sutra.protege;

import org.protege.editor.owl.ui.view.AbstractOWLViewComponent;
import org.semanticweb.owlapi.model.*;
import org.semanticweb.owlapi.formats.TurtleDocumentFormat;

import javax.swing.*;
import javax.swing.border.EmptyBorder;
import javax.swing.border.TitledBorder;
import java.awt.*;
import java.awt.event.ActionEvent;
import java.io.*;
import java.net.HttpURLConnection;
import java.net.URI;
import java.net.URL;
import java.nio.charset.StandardCharsets;

/**
 * Protégé view component that connects to a running SutraDB instance.
 *
 * Features:
 * - Check/start SutraDB server
 * - Load ontology from SutraDB into Protégé
 * - Save current ontology back to SutraDB
 * - Validate data against OWL constraints
 */
public class SutraDbView extends AbstractOWLViewComponent {

    private static final String DEFAULT_HOST = "localhost";
    private static final int DEFAULT_PORT = 3030;

    private JTextField hostField;
    private JTextField portField;
    private JLabel statusLabel;
    private JTextArea logArea;
    private JButton connectButton;
    private JButton loadButton;
    private JButton saveButton;
    private JButton validateButton;

    private Process sutraProcess;

    @Override
    protected void initialiseOWLView() throws Exception {
        setLayout(new BorderLayout(8, 8));

        // Connection panel
        JPanel connectionPanel = new JPanel(new FlowLayout(FlowLayout.LEFT, 8, 4));
        connectionPanel.setBorder(new TitledBorder("SutraDB Connection"));

        connectionPanel.add(new JLabel("Host:"));
        hostField = new JTextField(DEFAULT_HOST, 12);
        connectionPanel.add(hostField);

        connectionPanel.add(new JLabel("Port:"));
        portField = new JTextField(String.valueOf(DEFAULT_PORT), 5);
        connectionPanel.add(portField);

        statusLabel = new JLabel("  Not connected");
        statusLabel.setForeground(Color.GRAY);
        connectionPanel.add(statusLabel);

        add(connectionPanel, BorderLayout.NORTH);

        // Button panel
        JPanel buttonPanel = new JPanel(new FlowLayout(FlowLayout.LEFT, 8, 4));

        connectButton = new JButton("Connect / Start");
        connectButton.setToolTipText("Check if SutraDB is running; start it if not");
        connectButton.addActionListener(this::onConnect);
        buttonPanel.add(connectButton);

        loadButton = new JButton("Load from SutraDB");
        loadButton.setToolTipText("Load the ontology from SutraDB into Protégé");
        loadButton.addActionListener(this::onLoad);
        loadButton.setEnabled(false);
        buttonPanel.add(loadButton);

        saveButton = new JButton("Save to SutraDB");
        saveButton.setToolTipText("Export the current ontology as Turtle and upload to SutraDB");
        saveButton.addActionListener(this::onSave);
        saveButton.setEnabled(false);
        buttonPanel.add(saveButton);

        validateButton = new JButton("Validate");
        validateButton.setToolTipText("Run OWL verification queries against SutraDB");
        validateButton.addActionListener(this::onValidate);
        validateButton.setEnabled(false);
        buttonPanel.add(validateButton);

        add(buttonPanel, BorderLayout.CENTER);

        // Log area
        logArea = new JTextArea(8, 60);
        logArea.setEditable(false);
        logArea.setFont(new Font(Font.MONOSPACED, Font.PLAIN, 12));
        JScrollPane scrollPane = new JScrollPane(logArea);
        scrollPane.setBorder(new TitledBorder("Log"));
        add(scrollPane, BorderLayout.SOUTH);

        log("SutraDB plugin ready. Click 'Connect / Start' to begin.");
    }

    @Override
    protected void disposeOWLView() {
        // Clean up the SutraDB process if we started it
        if (sutraProcess != null && sutraProcess.isAlive()) {
            log("Shutting down SutraDB...");
            sutraProcess.destroy();
        }
    }

    private String getBaseUrl() {
        return "http://" + hostField.getText().trim() + ":" + portField.getText().trim();
    }

    private void log(String message) {
        SwingUtilities.invokeLater(() -> {
            logArea.append(message + "\n");
            logArea.setCaretPosition(logArea.getDocument().getLength());
        });
    }

    private void setConnected(boolean connected) {
        SwingUtilities.invokeLater(() -> {
            loadButton.setEnabled(connected);
            saveButton.setEnabled(connected);
            validateButton.setEnabled(connected);
            if (connected) {
                statusLabel.setText("  Connected");
                statusLabel.setForeground(new Color(0, 128, 0));
            } else {
                statusLabel.setText("  Not connected");
                statusLabel.setForeground(Color.GRAY);
            }
        });
    }

    // ── Connect / Start ─────────────────────────────────────────────────────

    private void onConnect(ActionEvent e) {
        new SwingWorker<Boolean, Void>() {
            @Override
            protected Boolean doInBackground() {
                // First try to connect
                if (checkHealth()) {
                    log("SutraDB is already running at " + getBaseUrl());
                    return true;
                }

                // Try to start it
                log("SutraDB not running. Attempting to start...");
                return tryStartSutra();
            }

            @Override
            protected void done() {
                try {
                    setConnected(get());
                } catch (Exception ex) {
                    log("Error: " + ex.getMessage());
                    setConnected(false);
                }
            }
        }.execute();
    }

    private boolean checkHealth() {
        try {
            URL url = URI.create(getBaseUrl() + "/health").toURL();
            HttpURLConnection conn = (HttpURLConnection) url.openConnection();
            conn.setConnectTimeout(2000);
            conn.setReadTimeout(2000);
            conn.setRequestMethod("GET");
            int code = conn.getResponseCode();
            conn.disconnect();
            return code == 200;
        } catch (Exception e) {
            return false;
        }
    }

    private boolean tryStartSutra() {
        try {
            // Try to find sutra executable
            String port = portField.getText().trim();
            ProcessBuilder pb = new ProcessBuilder("sutra", "serve", "--port", port);
            pb.redirectErrorStream(true);
            sutraProcess = pb.start();

            // Read output in background
            Thread outputReader = new Thread(() -> {
                try (BufferedReader br = new BufferedReader(
                        new InputStreamReader(sutraProcess.getInputStream(), StandardCharsets.UTF_8))) {
                    String line;
                    while ((line = br.readLine()) != null) {
                        log("[sutra] " + line);
                    }
                } catch (IOException ex) {
                    // Process ended
                }
            });
            outputReader.setDaemon(true);
            outputReader.start();

            // Wait for it to come up
            for (int i = 0; i < 20; i++) {
                Thread.sleep(500);
                if (checkHealth()) {
                    log("SutraDB started successfully on port " + port);
                    return true;
                }
            }

            log("SutraDB failed to start within 10 seconds.");
            log("Make sure 'sutra' is on your PATH, or start it manually.");
            return false;
        } catch (Exception e) {
            log("Could not start SutraDB: " + e.getMessage());
            log("Start it manually: sutra serve --port " + portField.getText().trim());
            return false;
        }
    }

    // ── Load from SutraDB ───────────────────────────────────────────────────

    private void onLoad(ActionEvent e) {
        new SwingWorker<Void, Void>() {
            @Override
            protected Void doInBackground() {
                try {
                    log("Loading ontology from " + getBaseUrl() + "/graph ...");

                    URL url = URI.create(getBaseUrl() + "/graph").toURL();
                    HttpURLConnection conn = (HttpURLConnection) url.openConnection();
                    conn.setRequestMethod("GET");
                    conn.setRequestProperty("Accept", "text/turtle");

                    int code = conn.getResponseCode();
                    if (code != 200) {
                        log("Error: server returned HTTP " + code);
                        return null;
                    }

                    // Read the Turtle content
                    String turtle;
                    try (InputStream is = conn.getInputStream()) {
                        turtle = new String(is.readAllBytes(), StandardCharsets.UTF_8);
                    }
                    conn.disconnect();

                    // Write to a temp file and load into Protégé
                    File tempFile = File.createTempFile("sutradb-", ".ttl");
                    tempFile.deleteOnExit();
                    try (Writer w = new OutputStreamWriter(
                            new FileOutputStream(tempFile), StandardCharsets.UTF_8)) {
                        w.write(turtle);
                    }

                    // Load via OWL API
                    OWLOntologyManager manager = getOWLModelManager().getOWLOntologyManager();
                    OWLOntology ont = manager.loadOntologyFromOntologyDocument(tempFile);
                    getOWLModelManager().setActiveOntology(ont);

                    int axiomCount = ont.getAxiomCount();
                    log("Loaded ontology: " + axiomCount + " axioms from SutraDB.");

                } catch (Exception ex) {
                    log("Load error: " + ex.getMessage());
                }
                return null;
            }
        }.execute();
    }

    // ── Save to SutraDB ─────────────────────────────────────────────────────

    private void onSave(ActionEvent e) {
        new SwingWorker<Void, Void>() {
            @Override
            protected Void doInBackground() {
                try {
                    log("Saving ontology to SutraDB...");

                    OWLOntology ont = getOWLModelManager().getActiveOntology();
                    OWLOntologyManager manager = getOWLModelManager().getOWLOntologyManager();

                    // Serialize to Turtle
                    ByteArrayOutputStream baos = new ByteArrayOutputStream();
                    manager.saveOntology(ont, new TurtleDocumentFormat(), baos);
                    String turtle = baos.toString(StandardCharsets.UTF_8);

                    // Convert Turtle to N-Triples (simple line-by-line approach)
                    // For now, POST the turtle as N-Triples to /triples
                    // This is a simplification — a proper implementation would
                    // use the OWL API's NTriples format instead
                    ByteArrayOutputStream ntStream = new ByteArrayOutputStream();
                    manager.saveOntology(ont,
                            new org.semanticweb.owlapi.formats.NTriplesDocumentFormat(),
                            ntStream);
                    String ntriples = ntStream.toString(StandardCharsets.UTF_8);

                    // POST to SutraDB
                    URL url = URI.create(getBaseUrl() + "/triples").toURL();
                    HttpURLConnection conn = (HttpURLConnection) url.openConnection();
                    conn.setRequestMethod("POST");
                    conn.setRequestProperty("Content-Type", "text/plain; charset=utf-8");
                    conn.setDoOutput(true);

                    try (OutputStream os = conn.getOutputStream()) {
                        os.write(ntriples.getBytes(StandardCharsets.UTF_8));
                    }

                    int code = conn.getResponseCode();
                    String response;
                    try (InputStream is = (code >= 400) ? conn.getErrorStream() : conn.getInputStream()) {
                        response = new String(is.readAllBytes(), StandardCharsets.UTF_8);
                    }
                    conn.disconnect();

                    if (code == 200) {
                        log("Saved to SutraDB: " + response);
                    } else {
                        log("Save error (HTTP " + code + "): " + response);
                    }

                } catch (Exception ex) {
                    log("Save error: " + ex.getMessage());
                }
                return null;
            }
        }.execute();
    }

    // ── Validate ────────────────────────────────────────────────────────────

    private void onValidate(ActionEvent e) {
        new SwingWorker<Void, Void>() {
            @Override
            protected Void doInBackground() {
                try {
                    log("Running OWL validation against SutraDB...");

                    OWLOntology ont = getOWLModelManager().getActiveOntology();
                    int violations = 0;

                    // Generate and run verification queries from OWL axioms
                    for (OWLAxiom axiom : ont.getAxioms()) {
                        String query = axiomToVerificationQuery(axiom);
                        if (query != null) {
                            String result = executeSparql(query);
                            if (result != null && !result.contains("\"bindings\":[]")) {
                                violations++;
                                log("VIOLATION [" + axiom.getAxiomType() + "]: " +
                                        shortenAxiom(axiom.toString()));
                            }
                        }
                    }

                    if (violations == 0) {
                        log("Validation passed — no constraint violations found.");
                    } else {
                        log("Validation complete: " + violations + " violation(s) found.");
                    }

                } catch (Exception ex) {
                    log("Validation error: " + ex.getMessage());
                }
                return null;
            }
        }.execute();
    }

    /**
     * Convert an OWL axiom to a SPARQL verification query.
     * Returns a query that finds VIOLATIONS (non-empty result = problem).
     * Returns null for axiom types we don't verify yet.
     */
    private String axiomToVerificationQuery(OWLAxiom axiom) {
        if (axiom instanceof OWLSubClassOfAxiom) {
            OWLSubClassOfAxiom sub = (OWLSubClassOfAxiom) axiom;
            OWLClassExpression superClass = sub.getSuperClass();
            OWLClassExpression subClass = sub.getSubClass();

            // Simple case: NamedClass subClassOf NamedClass
            // Violation: instance of subClass that is NOT instance of superClass
            if (!subClass.isAnonymous() && !superClass.isAnonymous()) {
                String subIri = subClass.asOWLClass().getIRI().toString();
                String superIri = superClass.asOWLClass().getIRI().toString();
                return "SELECT ?x WHERE { " +
                        "?x <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <" + subIri + "> . " +
                        "FILTER NOT EXISTS { ?x <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <" + superIri + "> } " +
                        "}";
            }
        }

        if (axiom instanceof OWLObjectPropertyDomainAxiom) {
            OWLObjectPropertyDomainAxiom dom = (OWLObjectPropertyDomainAxiom) axiom;
            if (!dom.getProperty().isAnonymous() && !dom.getDomain().isAnonymous()) {
                String propIri = dom.getProperty().asOWLObjectProperty().getIRI().toString();
                String domIri = dom.getDomain().asOWLClass().getIRI().toString();
                // Violation: subject of property that is NOT of domain type
                return "SELECT ?x WHERE { " +
                        "?x <" + propIri + "> ?y . " +
                        "FILTER NOT EXISTS { ?x <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <" + domIri + "> } " +
                        "}";
            }
        }

        if (axiom instanceof OWLObjectPropertyRangeAxiom) {
            OWLObjectPropertyRangeAxiom rng = (OWLObjectPropertyRangeAxiom) axiom;
            if (!rng.getProperty().isAnonymous() && !rng.getRange().isAnonymous()) {
                String propIri = rng.getProperty().asOWLObjectProperty().getIRI().toString();
                String rngIri = rng.getRange().asOWLClass().getIRI().toString();
                // Violation: object of property that is NOT of range type
                return "SELECT ?y WHERE { " +
                        "?x <" + propIri + "> ?y . " +
                        "FILTER NOT EXISTS { ?y <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <" + rngIri + "> } " +
                        "}";
            }
        }

        if (axiom instanceof OWLFunctionalObjectPropertyAxiom) {
            OWLFunctionalObjectPropertyAxiom func = (OWLFunctionalObjectPropertyAxiom) axiom;
            if (!func.getProperty().isAnonymous()) {
                String propIri = func.getProperty().asOWLObjectProperty().getIRI().toString();
                // Violation: subject has more than one value for this property
                return "SELECT ?x WHERE { " +
                        "?x <" + propIri + "> ?y1 . " +
                        "?x <" + propIri + "> ?y2 . " +
                        "FILTER(?y1 != ?y2) " +
                        "}";
            }
        }

        if (axiom instanceof OWLDisjointClassesAxiom) {
            OWLDisjointClassesAxiom disj = (OWLDisjointClassesAxiom) axiom;
            java.util.List<OWLClassExpression> classes = disj.getClassExpressionsAsList();
            if (classes.size() == 2 && !classes.get(0).isAnonymous() && !classes.get(1).isAnonymous()) {
                String iri1 = classes.get(0).asOWLClass().getIRI().toString();
                String iri2 = classes.get(1).asOWLClass().getIRI().toString();
                // Violation: instance of both classes
                return "SELECT ?x WHERE { " +
                        "?x <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <" + iri1 + "> . " +
                        "?x <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <" + iri2 + "> " +
                        "}";
            }
        }

        // Not yet supported
        return null;
    }

    private String executeSparql(String query) {
        try {
            URL url = URI.create(getBaseUrl() + "/sparql").toURL();
            HttpURLConnection conn = (HttpURLConnection) url.openConnection();
            conn.setRequestMethod("POST");
            conn.setRequestProperty("Content-Type", "application/sparql-query");
            conn.setDoOutput(true);

            try (OutputStream os = conn.getOutputStream()) {
                os.write(query.getBytes(StandardCharsets.UTF_8));
            }

            int code = conn.getResponseCode();
            String response;
            try (InputStream is = (code >= 400) ? conn.getErrorStream() : conn.getInputStream()) {
                response = new String(is.readAllBytes(), StandardCharsets.UTF_8);
            }
            conn.disconnect();

            return (code == 200) ? response : null;
        } catch (Exception e) {
            return null;
        }
    }

    private String shortenAxiom(String axiom) {
        if (axiom.length() > 120) {
            return axiom.substring(0, 117) + "...";
        }
        return axiom;
    }
}
