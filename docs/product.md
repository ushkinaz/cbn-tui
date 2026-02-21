# Initial Concept
TUI browser for '[Cataclysm: Bright Nights](https://github.com/cataclysmbn/Cataclysm-BN)' JSON data.

# Product Definition

## Target Audience
- **Game Modders:** Users who need to quickly verify JSON definitions for their mods.
- **Tool Developers:** Developers building tools for C:BN who need to explore and understand the game's complex JSON structure.

## Goals
- **High-Performance Search:** Provide an ultra-fast, lag-free search experience for massive (tens of MBs) JSON datasets.
- **Interactive Exploration:** Offer a visual and interactive way to navigate and understand complex nested JSON structures without needing an IDE or external editor.

## Key Features
- **Advanced Search Syntax:** Support for field-specific filtering, dot-notation for deep field access, and combined search logic.

## User Stories
- **Deep Field Browsing:** As a developer, I want to use dot-notation (e.g., `bash.str_min:10`) to find specific items and understand how their properties are structured within the game's JSON.

## Non-Functional Requirements
- **Performance:** Zero-lag searching and rendering even with large JSON files.
- **Portability:** Seamless operation across Windows, macOS, and Linux platforms.
