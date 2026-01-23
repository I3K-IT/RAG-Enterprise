# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 1.x.x   | :white_check_mark: |
| < 1.0   | :x:                |

## Reporting a Vulnerability

We take the security of RAG Enterprise seriously. If you believe you have found a security vulnerability, please report it to us as described below.

### How to Report

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, please report them via email to: **info@i3k.eu**

You should receive a response within 48 hours. If for some reason you do not, please follow up via email to ensure we received your original message.

Please include the following information in your report:

- Type of issue (e.g., buffer overflow, SQL injection, cross-site scripting, etc.)
- Full paths of source file(s) related to the issue
- Location of the affected source code (tag/branch/commit or direct URL)
- Any special configuration required to reproduce the issue
- Step-by-step instructions to reproduce the issue
- Proof-of-concept or exploit code (if possible)
- Impact of the issue, including how an attacker might exploit it

### What to Expect

- **Acknowledgment**: We will acknowledge receipt of your report within 48 hours
- **Communication**: We will keep you informed of the progress towards a fix
- **Credit**: We will credit you in the security advisory (unless you prefer to remain anonymous)
- **Timeline**: We aim to resolve critical issues within 7 days

## Security Best Practices

When deploying RAG Enterprise, please follow these security guidelines:

### 1. Authentication

- **Always set a strong JWT_SECRET_KEY** in production
  ```bash
  openssl rand -hex 32
  ```
- **Change the default admin password** immediately after first login
- Use strong passwords for all user accounts

### 2. Network Security

- **Configure ALLOWED_ORIGINS** to restrict API access
  ```env
  ALLOWED_ORIGINS=https://your-domain.com
  ```
- Use HTTPS in production (configure a reverse proxy like Nginx or Caddy)
- Keep the Qdrant database on an internal network

### 3. File Upload Security

- The system validates file extensions and sizes
- Configure `MAX_UPLOAD_SIZE_MB` appropriately for your needs
- Consider additional malware scanning for uploaded documents

### 4. Database Security

- SQLite database is stored in `/app/data/`
- Ensure proper file permissions on the data volume
- For production, consider migrating to PostgreSQL

### 5. Docker Security

- Run containers with minimal privileges
- Keep Docker and all images updated
- Use Docker secrets for sensitive configuration

### 6. Monitoring

- Monitor backend logs for suspicious activity
- Set up alerts for failed login attempts
- Regularly review user access

## Known Security Considerations

### Local LLM (Ollama)

- Ollama runs locally, so queries don't leave your network
- The LLM model has access to all indexed documents
- Consider network isolation for sensitive deployments

### Vector Database (Qdrant)

- Configure `QDRANT_API_KEY` for production
- Qdrant stores document embeddings (not raw text)
- Embeddings could potentially be reverse-engineered

### Document Processing

- OCR processes documents server-side
- Temporary files are created during processing
- Ensure proper cleanup of uploaded files

## Security Updates

Security updates will be released as patch versions (e.g., 1.0.1) and announced via:

- GitHub Security Advisories
- Release notes
- Project README

We recommend enabling GitHub notifications for this repository to stay informed about security updates.
