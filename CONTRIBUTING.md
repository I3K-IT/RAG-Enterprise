# Contributing to RAG Enterprise

First off, thank you for considering contributing to RAG Enterprise! It's people like you that make RAG Enterprise such a great tool.

## Code of Conduct

This project and everyone participating in it is governed by our [Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code.

## How Can I Contribute?

### Reporting Bugs

Before creating bug reports, please check the existing issues to avoid duplicates. When you create a bug report, include as many details as possible:

- **Use a clear and descriptive title**
- **Describe the exact steps to reproduce the problem**
- **Provide specific examples** (config files, documents that cause issues, etc.)
- **Describe the behavior you observed and what you expected**
- **Include logs** from `docker compose logs backend`
- **Specify your environment** (OS, Docker version, GPU model if applicable)

### Suggesting Enhancements

Enhancement suggestions are tracked as GitHub issues. When creating an enhancement suggestion:

- **Use a clear and descriptive title**
- **Provide a detailed description** of the suggested enhancement
- **Explain why this enhancement would be useful** to most users
- **List any alternatives you've considered**

### Pull Requests

1. **Fork the repository** and create your branch from `main`
2. **Follow the coding style** of the project
3. **Add tests** if you're adding new functionality
4. **Update documentation** if needed
5. **Ensure the test suite passes**
6. **Make sure your code lints**

## Development Setup

### Prerequisites

- Docker & Docker Compose
- NVIDIA GPU with CUDA support (recommended)
- Git

### Local Development

```bash
# Clone your fork
git clone https://github.com/YOUR_USERNAME/RAG-Enterprise.git
cd RAG-Enterprise

# Create a branch for your feature
git checkout -b feature/your-feature-name

# Start the development environment
cd rag-enterprise-structure
./setup.sh

# Make your changes...

# Test your changes
docker compose logs -f backend
```

### Project Structure

```
RAG-Enterprise/
├── frontend/                 # React frontend (Vite)
│   └── src/App.jsx          # Main application
├── rag-enterprise-structure/
│   ├── backend/             # FastAPI backend
│   │   ├── app.py           # Main API
│   │   ├── rag_pipeline.py  # RAG logic
│   │   ├── ocr_service.py   # Document processing
│   │   └── ...
│   ├── docker-compose.yml   # Container orchestration
│   └── .env.example         # Configuration template
└── docs/                    # Documentation
```

### Backend Development

The backend is built with FastAPI. Key files:

| File | Purpose |
|------|---------|
| `app.py` | API endpoints and request handling |
| `rag_pipeline.py` | RAG query processing |
| `embeddings_service.py` | Text embedding generation |
| `ocr_service.py` | Document text extraction |
| `qdrant_connector.py` | Vector database operations |
| `auth.py` | JWT authentication |
| `database.py` | User management (SQLite) |

### Frontend Development

The frontend is a React SPA built with Vite:

```bash
cd frontend
npm install
npm run dev
```

## Coding Guidelines

### Python (Backend)

- Follow PEP 8 style guide
- Use type hints for function parameters and return values
- Write docstrings for public functions
- Keep functions focused and small
- Use meaningful variable names

```python
# Good
def process_document(file_path: str, chunk_size: int = 1000) -> List[str]:
    """
    Process a document and return text chunks.

    Args:
        file_path: Path to the document file
        chunk_size: Maximum characters per chunk

    Returns:
        List of text chunks
    """
    ...

# Bad
def proc(f, s=1000):
    ...
```

### JavaScript/React (Frontend)

- Use functional components with hooks
- Use meaningful component and variable names
- Keep components small and focused
- Use PropTypes or TypeScript for type checking

### Commit Messages

Use clear, descriptive commit messages:

```
<emoji> <type>: <subject>

<body>
```

Types and emojis:
- `feat` - New feature
- `fix` - Bug fix
- `docs` - Documentation
- `style` - Formatting, no code change
- `refactor` - Code restructuring
- `test` - Adding tests
- `chore` - Maintenance

Example:
```
feat: Add PDF password protection support

- Implement password detection in OCR service
- Add password input modal in frontend
- Update documentation with new feature
```

## Testing

### Running Tests

```bash
# Backend tests
cd rag-enterprise-structure/backend
pytest

# Frontend tests
cd frontend
npm test
```

### Writing Tests

- Write tests for new features
- Ensure existing tests pass
- Aim for meaningful test coverage

## Documentation

- Update README.md if adding new features
- Add inline comments for complex logic
- Update .env.example for new configuration options

## Questions?

Feel free to open an issue with the `question` label or start a discussion.

## License

By contributing, you agree that your contributions will be licensed under the AGPL-3.0 License.
