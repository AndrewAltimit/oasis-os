# OpenCode Container Scripts

This directory contains scripts for running OpenCode AI assistant in Docker containers.

## Quick Start

### Option 1: Simple Standalone Container (Recommended for beginners)

The simplest way to get started - no docker compose required:

```bash
./run_opencode_simple.sh
```

This script:
- Automatically builds a lightweight OpenCode container
- Prompts for API key if not set
- Works out of the box with minimal dependencies
- Perfect for quick code generation tasks

### Option 2: Full Infrastructure Container

For users who need the complete MCP infrastructure:

```bash
./run_opencode_container.sh
```

This script:
- Needs the `template-repo-mcp-opencode`, `template-repo-mcp-crush` and
  `template-repo-openrouter-agents` images, which are built from the upstream
  template repository these scripts were copied from (this repo ships neither
  their Dockerfiles nor a `build:` section for them; see Troubleshooting)
- Integrates with the full docker compose stack
- Provides access to additional MCP tools
- Suitable for advanced workflows

## Features Comparison

| Feature | Simple Container | Full Infrastructure |
|---------|-----------------|-------------------|
| Setup Time | ~1 minute | ~5 minutes |
| Dependencies | Docker only | Docker + docker compose |
| Image Size | ~300MB | ~2GB |
| MCP Tools | No | Yes |
| Auto-build | Yes | No (prebuilt images) |
| Interactive Mode | Yes | Yes |
| Single Query Mode | Yes | Yes |

## Environment Variables

Both scripts support:
- `OPENROUTER_API_KEY` - Your OpenRouter API key (required)
- `OPENCODE_MODEL` - Model to use (default: qwen/qwen-2.5-coder-32b-instruct)

## Usage Examples

### Interactive Mode
```bash
# Simple
./run_opencode_simple.sh

# Full
./run_opencode_container.sh
```

### Single Query Mode
```bash
# Simple
./run_opencode_simple.sh -q "Write a Python fibonacci function"

# Full
./run_opencode_container.sh -q "Write a Python fibonacci function"
```

### With Context File
```bash
# Full infrastructure only
./run_opencode_container.sh -q "Refactor this code" -c existing_code.py
```

## Troubleshooting

### Images Not Building

The full-infrastructure script cannot build its images in this repository:
`docker/openrouter-agents.Dockerfile` and the Dockerfiles behind the
`mcp-opencode` / `mcp-crush` compose services are not part of OASIS_OS (the
compose file only names the `template-repo-mcp-*:latest` images). Either:
1. Build or pull those three images from the upstream template repository,
   tagged `template-repo-mcp-opencode:latest`, `template-repo-mcp-crush:latest`
   and `template-repo-openrouter-agents:latest`, then rerun the script; or
2. Use `./run_opencode_simple.sh`, which builds its own image here.

`run_opencode_container.sh` stops with this explanation when an image is
missing instead of attempting the build.

### API Key Issues

If you get API key errors:
1. Get a key from https://openrouter.ai/keys
2. Export it: `export OPENROUTER_API_KEY='your-key-here'`
3. Or add to `.env` file in project root

### Permission Errors

If you get permission errors:
1. Ensure scripts are executable: `chmod +x *.sh`
2. Check Docker permissions: `docker ps`

## Company Integration

For the company-specific OpenCode integration with proxy support, see:
- `/automation/corporate-proxy/` - Corporate proxy integrations for Crush and OpenCode
- `/OPENCODE_COMPANY_INTEGRATION.md` - Full documentation
