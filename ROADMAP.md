# RAG Enterprise Roadmap

This document outlines the planned features and improvements for RAG Enterprise.

## Current Version: 1.0.0

### Completed
- [x] One-command setup with Docker Compose
- [x] Multi-format document processing (PDF, DOCX, images, etc.)
- [x] Local LLM support via Ollama
- [x] Vector search with Qdrant
- [x] JWT authentication with role-based access
- [x] Conversational memory per user
- [x] GPU acceleration support
- [x] OCR for scanned documents

---

## Short Term (v1.1.0)

### Performance
- [ ] Batch document processing
- [ ] Async embedding generation
- [ ] Response streaming

### Features
- [ ] Document tagging and categories
- [ ] Advanced search filters
- [ ] Export conversations to PDF/Markdown
- [ ] Dark mode UI

### Security
- [ ] Two-factor authentication (2FA)
- [ ] API rate limiting
- [ ] Audit logging

---

## Medium Term (v1.2.0)

### Scalability
- [ ] PostgreSQL support (replace SQLite)
- [ ] Redis caching layer
- [ ] Horizontal scaling guide
- [ ] Kubernetes deployment manifests

### AI Improvements
- [ ] Multiple LLM provider support (OpenAI, Anthropic, etc.)
- [ ] Custom embedding models
- [ ] RAG evaluation metrics
- [ ] Hybrid search (keyword + semantic)

### Integrations
- [ ] Webhook notifications
- [ ] REST API v2 with OpenAPI 3.1
- [ ] Python SDK
- [ ] CLI tool

---

## Long Term (v2.0.0)

### Enterprise Features
- [ ] Multi-tenant architecture
- [ ] SSO/SAML integration
- [ ] Advanced analytics dashboard
- [ ] Compliance reporting (GDPR, HIPAA)

### Advanced RAG
- [ ] Multi-modal RAG (images, tables)
- [ ] Knowledge graph integration
- [ ] Automatic document summarization
- [ ] Citation verification

### Platform
- [ ] Plugin system
- [ ] Custom workflow builder
- [ ] Mobile app (iOS/Android)

---

## Community Wishlist

Features requested by the community (vote with reactions!):

| Feature | Votes | Status |
|---------|-------|--------|
| API key authentication | - | Planned |
| S3/MinIO storage | - | Under review |
| Multi-language UI | - | Under review |
| Ollama model manager | - | Under review |

---

## Contributing

Want to help implement a feature? Check our [Contributing Guide](CONTRIBUTING.md)!

Priorities may change based on community feedback. Open an issue to suggest new features or vote on existing ones.
