package io.github.emmaleonhart.sutradb;

/**
 * Exception thrown when a SutraDB operation fails.
 *
 * <p>Wraps both HTTP-level errors (connection failures, timeouts) and
 * application-level errors returned by the SutraDB server.</p>
 */
public class SutraError extends RuntimeException {

    private final int statusCode;

    /**
     * Create a new SutraError with a message and HTTP status code.
     *
     * @param message    human-readable error description
     * @param statusCode the HTTP status code, or -1 if the error is not HTTP-related
     */
    public SutraError(String message, int statusCode) {
        super(message);
        this.statusCode = statusCode;
    }

    /**
     * Create a new SutraError wrapping another throwable.
     *
     * @param message human-readable error description
     * @param cause   the underlying cause
     */
    public SutraError(String message, Throwable cause) {
        super(message, cause);
        this.statusCode = -1;
    }

    /**
     * Return the HTTP status code associated with this error,
     * or -1 if the error is not HTTP-related.
     *
     * @return HTTP status code or -1
     */
    public int getStatusCode() {
        return statusCode;
    }
}
