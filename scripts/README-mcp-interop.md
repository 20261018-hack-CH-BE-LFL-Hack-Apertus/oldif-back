# Check read-only MCP interoperability

Use an explicit installation of `@modelcontextprotocol/sdk@1.32.0` with Node.js. This optional test client is not a backend runtime dependency.

```sh
npm install --prefix /tmp/oldif-mcp-client --ignore-scripts @modelcontextprotocol/sdk@1.32.0
OLDIF_MCP_SDK_DIR=/tmp/oldif-mcp-client/node_modules/@modelcontextprotocol/sdk \
OLDIF_BACKEND_URL=http://127.0.0.1:8080 \
OLDIF_APP_ORIGIN=http://localhost:3000 \
OLDIF_MCP_RECEIPT=/tmp/oldif-mcp-check.json \
node scripts/check_mcp_interop.mjs
```

The client creates a temporary demo identity, negotiates the server's existing protocol, lists the two read-only tools, performs actual Stapfer search/source requests, and checks that unknown tools and extra arguments are rejected. It does not call an inference provider. The API bounds tool outputs to 256 KiB, six search candidates and thirty tool calls per identity per minute. Larger records remain available through their authenticated archive routes.

The test covers this bounded stateless tool subset. It does not claim complete support for every MCP transport, authorization extension, server notification, resource subscription or task feature.
