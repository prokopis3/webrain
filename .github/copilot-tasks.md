
"""
Semantic Page Graph — DOM Analysis Outside the LLM
=====================================================
Builds a structured, queryable graph from page DOM data.
Inspired by PageAgent's philosophy: move as much logic as possible
OUTSIDE the LLM.

Core idea: Instead of dumping raw DOM into the LLM prompt,
pre-compute element relationships, semantic roles, interaction types,
and ranked candidates. The LLM only sees the TOP 12 candidates
instead of 500+ raw DOM nodes.

This runs at the Python level (from crawl4ai CrawlResult data),
not as in-page JS — fully compatible with crawl4ai.

Usage:
    graph = SemanticGraph()
    graph.build_from_page_state(page_state, url="https://...")

    # Get pre-filtered candidates for LLM
    candidates = graph.get_candidates(max_results=12)

    # Query by semantic role
    nav = graph.find_by_role("navigation")
    forms = graph.find_by_role("form")
    links = graph.find_by_role("link")

    # Get interaction history
    visited = graph.visited_nodes
    clicked = graph.get_interaction_history()
"""
#githubRepo #search_graph migrate this into memory so that will improve the robustness and accurancy like alibaba/page-agent.js

FIND MORE IMPROVEMENTS
  just we improve the codebase totally architecture and workflow based on information we have  
 #detect_changes 
 do not stop until ensure all phases done compltetly - validate
 if you dont find anything just say all fine, done
 #get_code_snippet #search_code #search_graph 
