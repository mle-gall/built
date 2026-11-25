# What works today

Built is pre-release. This is what the current build does.

## Modelling

- Line, arc, rectangle, circle, polygon, push/pull, offset, follow-me, move, rotate, scale, fillet, solid operations, tape, protractor, dimensions and text.
- Point, edge, face and axis inference while you draw.
- Section planes with fills and hatches, scenes, and a sheet composer for drawings.

## Large models

- Only what is in view is drawn, and the interface is drawn in the same frame as the model.
- Models of several million faces at survey coordinates stay workable.

## Project tree

- One tree with three column sets: Filing, Drawing and Export.
- A node carries a recipe: IFC class and type, tag, classification and default properties. The elements filed under it follow.
- The Drawing columns set line weight, hatch and colour per node.

![The Drawing columns of the project tree: line weight, hatch, line and colour per node](images/built/drawing-columns.png)

## Records and checks

- Each object has a record stored in the file.
- Problems are listed per node before export, with a checklist of what to fix.

## Linked files

- A project can link other documents, nested links included, and reloads them when the source changes.

## Scripting

- Python plugins and an agent bridge (MCP) work on the same documents. See [`plugins.md`](plugins.md) and [`ai-bridge.md`](ai-bridge.md).

## AI

- The AI Assistant panel talks to the provider you configure with your own key: Anthropic, OpenAI, Google Gemini, Groq, OpenRouter, DeepSeek, OpenCode Go, or a local OpenAI-compatible server such as Ollama or LM Studio.
- The MCP bridge lets your own agent work on the open document. It listens on your machine only. See [`ai-bridge.md`](ai-bridge.md).
- An agent can query the model, groups, tags, records, registry and project tree, set the camera and take viewport screenshots, run Python on the live document, file elements, edit records and classification, and export IFC.
- Every action is one undo step. Code that fails is rolled back whole.

## Drawings and sheets

- The Sheets workspace places views of the model on paper at a scale and exports PDF, DXF, SVG or an image.

![The Sheets workspace with one A4 landscape sheet](images/built/sheets.png)

## Import and export

- Imports: `.skp`, COLLADA, DXF, glTF/GLB, OBJ, STL. `.dwg` needs an external converter; `.rfa` needs its reader built.
- Exports: IFC, glTF/GLB, COLLADA, OBJ, STL, and the current view as DXF, SVG or an image.
- Rendering through Blender, when Blender is installed.

## Formats

| Format | Support |
|---|---|
| `.built` | Built's own format: a ZIP with JSON and binary geometry, 64-bit coordinates. See [`built-format.md`](built-format.md). |
| `.igz` | Opens. These are the files of the modeller Built grew from. |
| `.skp` | Imports `.skp` files, with their materials. |
| IFC | Exports IFC4 (and IFC4X3) from the object records. IDS checks need `ifcopenshell`, which `requirements.txt` does not install. See [`ifc-export.md`](ifc-export.md). |

## Architecture

Native Rust, with a GPU renderer.
