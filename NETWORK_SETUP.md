# 🌐 RAG Enterprise - Network Access Configuration

Complete guide to configure RAG Enterprise for access from **local area network (LAN)** or **internet**.

---

## 📋 Prerequisites

- Docker and Docker Compose installed
- RAG Enterprise already working on localhost
- Router access (only for internet configuration)

---

## 🏠 Scenario 1: LOCALHOST Access (Default)

**When to use:** Local development, testing on single computer.

**Configuration:** Already active by default, no changes required.

```bash
# System is accessible only from:
http://localhost:3000
```

---

## 🏢 Scenario 2: LOCAL NETWORK Access (LAN)

**When to use:** Access from other devices on the same network (e.g., office, home).

### 📝 Step 1: Find the server IP

On the server where Docker is running, execute:

```bash
# Linux/Mac
ip addr show | grep "inet " | grep -v 127.0.0.1

# Windows
ipconfig | findstr IPv4
```

**Example output:** `192.168.1.100`

### 📝 Step 2: Modify the `.env` file

Open the `.env` file in the project root:

```bash
cd /path/to/rag-enterprise
nano .env
```

Modify the `VITE_API_URL` line:

```bash
# Before (localhost)
VITE_API_URL=http://localhost:8000

# After (LAN) - replace with YOUR IP
VITE_API_URL=http://192.168.1.100:8000
```

### 📝 Step 3: Rebuild and restart the containers

```bash
cd rag-enterprise-structure

# Rebuild the frontend with the new configuration
docker compose build frontend

# Restart all services
docker compose up -d
```

### 📝 Step 4: Test access

From another device on the same network, open the browser and go to:

```
http://192.168.1.100:3000
```

**✅ Done!** Now you can access from any device on your local network.

---

### ⚠️ Firewall

If you cannot access, it might be the server firewall. Open the necessary ports:

```bash
# Ubuntu/Debian
sudo ufw allow 3000/tcp
sudo ufw allow 8000/tcp

# CentOS/RHEL
sudo firewall-cmd --permanent --add-port=3000/tcp
sudo firewall-cmd --permanent --add-port=8000/tcp
sudo firewall-cmd --reload
```

---

## 🌍 Scenario 3: INTERNET Access (Advanced)

**When to use:** Public access from anywhere with internet connection.

### ⚠️ Requirements

1. **Domain** (e.g., yourdomain.com) or Dynamic DNS
2. **Static public IP** or Dynamic DNS service
3. **Reverse proxy** (Nginx) with SSL
4. **Port forwarding** on router

### 🏗️ Recommended Architecture

```
Internet
   ↓
Router (Port Forwarding: 80→Nginx, 443→Nginx)
   ↓
Nginx Reverse Proxy (SSL/TLS)
   ↓
Docker Containers (Frontend:3000, Backend:8000)
```

---

### 📝 Step 1: Configure Nginx Reverse Proxy

Install Nginx on the server:

```bash
# Ubuntu/Debian
sudo apt update
sudo apt install nginx

# CentOS/RHEL
sudo yum install nginx
```

Create the configuration file:

```bash
sudo nano /etc/nginx/sites-available/rag-enterprise
```

Paste this configuration (replace `yourdomain.com` with your domain):

```nginx
# HTTP (redirect to HTTPS)
server {
    listen 80;
    server_name yourdomain.com www.yourdomain.com;

    # Redirect to HTTPS
    return 301 https://$server_name$request_uri;
}

# HTTPS
server {
    listen 443 ssl http2;
    server_name yourdomain.com www.yourdomain.com;

    # SSL Certificate (use Certbot to get them for free)
    ssl_certificate /etc/letsencrypt/live/yourdomain.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/yourdomain.com/privkey.pem;

    # SSL Configuration
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers HIGH:!aNULL:!MD5;
    ssl_prefer_server_ciphers on;

    # Frontend (React)
    location / {
        proxy_pass http://localhost:3000;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection 'upgrade';
        proxy_set_header Host $host;
        proxy_cache_bypass $http_upgrade;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    # Backend API
    location /api/ {
        proxy_pass http://localhost:8000/api/;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # Timeout for LLM queries (can take time)
        proxy_read_timeout 300s;
        proxy_connect_timeout 300s;
        proxy_send_timeout 300s;
    }

    # Health check
    location /health {
        proxy_pass http://localhost:8000/health;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
    }

    # Upload files (longer timeout)
    location /api/documents/upload {
        proxy_pass http://localhost:8000/api/documents/upload;
        proxy_http_version 1.1;
        client_max_body_size 100M;
        proxy_read_timeout 600s;
        proxy_connect_timeout 600s;
    }
}
```

Enable the site:

```bash
# Ubuntu/Debian
sudo ln -s /etc/nginx/sites-available/rag-enterprise /etc/nginx/sites-enabled/
sudo nginx -t
sudo systemctl restart nginx

# CentOS/RHEL
sudo nginx -t
sudo systemctl restart nginx
```

---

### 📝 Step 2: Obtain SSL Certificate with Let's Encrypt

```bash
# Install Certbot
sudo apt install certbot python3-certbot-nginx  # Ubuntu/Debian
# or
sudo yum install certbot python3-certbot-nginx  # CentOS/RHEL

# Obtain free SSL certificate
sudo certbot --nginx -d yourdomain.com -d www.yourdomain.com

# Certbot will automatically configure Nginx with HTTPS
```

The certificate will renew automatically. Verify with:

```bash
sudo certbot renew --dry-run
```

---

### 📝 Step 3: Configure Port Forwarding on Router

Access your router panel (e.g., 192.168.1.1) and create these rules:

| External Port | Internal Port | Internal IP      | Protocol |
|---------------|---------------|------------------|----------|
| 80            | 80            | 192.168.1.100    | TCP      |
| 443           | 443           | 192.168.1.100    | TCP      |

*(Replace 192.168.1.100 with your server IP)*

---

### 📝 Step 4: Update `.env` for Internet

```bash
cd /path/to/rag-enterprise
nano .env
```

Modify:

```bash
# Internet with SSL
VITE_API_URL=https://yourdomain.com/api
```

Rebuild:

```bash
cd rag-enterprise-structure
docker compose build frontend
docker compose up -d
```

---

### 📝 Step 5: Verify Access

From any device connected to internet, open:

```
https://yourdomain.com
```

**✅ Done!** Your system is accessible from internet with SSL.

---

## 🔒 Security - Best Practices

### 1. **Change Default Password**

After first access, **immediately change** the default password:

- Username: `admin`
- Password: `admin123`

Go to: **🔑 Change Password** in the interface.

### 2. **Use Strong Passwords**

- Minimum 12 characters
- Upper and lowercase letters
- Numbers and symbols

### 3. **Limit Admin Access**

Create users with `user` or `super_user` role for daily operations.

### 4. **Database Backup**

The user database is in:

```bash
rag-enterprise-structure/backend/data/rag_users.db
```

Make regular backups:

```bash
# Backup
cp rag-enterprise-structure/backend/data/rag_users.db ~/backup/rag_users_$(date +%Y%m%d).db

# Restore (if needed)
cp ~/backup/rag_users_YYYYMMDD.db rag-enterprise-structure/backend/data/rag_users.db
docker compose restart backend
```

### 5. **Restrictive Firewall**

If using LAN, block internet access:

```bash
# Allow only from LAN
sudo ufw allow from 192.168.1.0/24 to any port 3000
sudo ufw allow from 192.168.1.0/24 to any port 8000
```

### 6. **Monitor Access**

Check the logs:

```bash
# Backend log
docker compose logs backend --tail 100

# Nginx log (if used)
sudo tail -f /var/log/nginx/access.log
```

---

## 🐛 Troubleshooting

### Problem: "net::ERR_CONNECTION_REFUSED"

**Cause:** Frontend cannot contact the backend.

**Solution:**
1. Verify that `VITE_API_URL` in `.env` is correct
2. Rebuild the frontend: `docker compose build frontend`
3. Verify that backend is active: `docker compose ps`

### Problem: "401 Unauthorized" on all requests

**Cause:** JWT token expired or invalid.

**Solution:**
1. Logout and login
2. Verify that server time is correct: `date`

### Problem: CORS errors

**Cause:** Backend blocks requests from different origins.

**Solution:**
Backend has `allow_origins=["*"]` by default (see `app.py:161`). If needed, modify:

```python
app.add_middleware(
    CORSMiddleware,
    allow_origins=["http://yourdomain.com", "https://yourdomain.com"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)
```

### Problem: File upload fails

**Cause:** Timeout too short or file too large.

**Solution:**
1. Increase Nginx timeout (see configuration above)
2. Increase `client_max_body_size` in Nginx

---

## 📞 Support

For problems or questions:
- Check logs: `docker compose logs backend --tail 100`
- Verify health: `curl http://localhost:8000/health`
- GitHub Issues: (link to your repository)

---

## 🎯 Configuration Summary

| Scenario | `.env` VITE_API_URL | Accessible From |
|----------|--------------------------|----------------|
| **Localhost** | `http://localhost:8000` | This computer only |
| **LAN** | `http://192.168.1.100:8000` | Devices on local network |
| **Internet** | `https://yourdomain.com/api` | Anywhere with internet |

---

**Done!** 🎉 Your RAG Enterprise system is configured for network access.
