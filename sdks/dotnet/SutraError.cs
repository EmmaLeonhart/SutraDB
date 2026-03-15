namespace SutraDB.Client;

/// <summary>
/// Exception thrown when a SutraDB operation fails.
/// </summary>
public class SutraException : Exception
{
    /// <summary>
    /// The HTTP status code returned by the server, or null if the error is not HTTP-related.
    /// </summary>
    public int? StatusCode { get; }

    /// <summary>
    /// Create a new SutraException with a message and optional status code.
    /// </summary>
    public SutraException(string message, int? statusCode = null)
        : base(message)
    {
        StatusCode = statusCode;
    }

    /// <summary>
    /// Create a new SutraException wrapping an inner exception.
    /// </summary>
    public SutraException(string message, Exception innerException)
        : base(message, innerException)
    {
        StatusCode = null;
    }
}
