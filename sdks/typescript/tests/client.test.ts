import { SutraClient, SutraError } from "../src";

describe("SutraClient", () => {
  it("uses default endpoint when none is provided", () => {
    const client = new SutraClient();
    // Access private field via index signature for testing purposes.
    expect((client as any).endpoint).toBe("http://localhost:3030");
  });

  it("accepts a custom endpoint", () => {
    const client = new SutraClient("http://db.example.com:9999");
    expect((client as any).endpoint).toBe("http://db.example.com:9999");
  });

  it("strips trailing slashes from the endpoint", () => {
    const client = new SutraClient("http://localhost:3030/");
    expect((client as any).endpoint).toBe("http://localhost:3030");
  });

  it("exposes health as an async function", () => {
    const client = new SutraClient();
    expect(typeof client.health).toBe("function");
  });

  it("exposes sparql as an async function", () => {
    const client = new SutraClient();
    expect(typeof client.sparql).toBe("function");
  });

  it("exposes insertTriples as an async function", () => {
    const client = new SutraClient();
    expect(typeof client.insertTriples).toBe("function");
  });

  it("exposes declareVector as an async function", () => {
    const client = new SutraClient();
    expect(typeof client.declareVector).toBe("function");
  });

  it("exposes insertVector as an async function", () => {
    const client = new SutraClient();
    expect(typeof client.insertVector).toBe("function");
  });
});

describe("SutraError", () => {
  it("is an instance of Error", () => {
    const err = new SutraError("test");
    expect(err).toBeInstanceOf(Error);
    expect(err).toBeInstanceOf(SutraError);
  });

  it("stores the message", () => {
    const err = new SutraError("something went wrong");
    expect(err.message).toBe("something went wrong");
  });

  it("stores an optional status code", () => {
    const err = new SutraError("not found", 404);
    expect(err.statusCode).toBe(404);
  });

  it("has statusCode undefined when not provided", () => {
    const err = new SutraError("oops");
    expect(err.statusCode).toBeUndefined();
  });

  it("has the correct name", () => {
    const err = new SutraError("test");
    expect(err.name).toBe("SutraError");
  });
});
