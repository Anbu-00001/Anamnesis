"""Drive Anamnesis from a LangGraph agent — through the existing MCP server.

The point of this example: there is *no bespoke LangChain binding to maintain*.
The `ana mcp` server already speaks the Model Context Protocol, and
`langchain-mcp-adapters` turns any MCP server's tools into LangChain/LangGraph
tools automatically. So a LangGraph agent gets predict / resolve / calibration /
list as first-class tools for free.

Run:
    pip install "anamnesis[langchain]"        # langchain-mcp-adapters + langgraph
    # (and an LLM provider, e.g. langchain-anthropic, plus its API key)
    ana --version                              # the `ana` binary must be on PATH
    python examples/langgraph_mcp.py

This file is intentionally not exercised by the test suite (it needs network and
an LLM). It documents the integration path.
"""
import asyncio


async def main() -> None:
    from langchain_mcp_adapters.client import MultiServerMCPClient
    from langgraph.prebuilt import create_react_agent

    # Launch `ana mcp` as a stdio MCP server and adapt its tools. Point --data at
    # whichever ledger this agent should keep score in.
    client = MultiServerMCPClient(
        {
            "anamnesis": {
                "command": "ana",
                "args": ["mcp"],  # add ["--data", "/path/to/agent.json"] to pick a ledger
                "transport": "stdio",
            }
        }
    )

    tools = await client.get_tools()
    print("Anamnesis tools exposed to LangGraph:", [t.name for t in tools])

    # Any chat model works; swap for your provider of choice.
    agent = create_react_agent("anthropic:claude-sonnet-4-6", tools)

    result = await agent.ainvoke(
        {
            "messages": [
                {
                    "role": "user",
                    "content": (
                        "Before I start: log a prediction that this refactor's tests "
                        "pass on the first try, at 70% confidence. Then show my "
                        "current calibration."
                    ),
                }
            ]
        }
    )
    print(result["messages"][-1].content)


if __name__ == "__main__":
    asyncio.run(main())
