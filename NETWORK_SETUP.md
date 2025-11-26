# 🌐 RAG Enterprise - Configurazione Accesso Network

Guida completa per configurare RAG Enterprise per l'accesso da **rete locale (LAN)** o da **internet**.

---

## 📋 Prerequisiti

- Docker e Docker Compose installati
- RAG Enterprise già funzionante su localhost
- Accesso al router (solo per configurazione internet)

---

## 🏠 Scenario 1: Accesso LOCALHOST (Default)

**Quando usarlo:** Sviluppo locale, test sul singolo computer.

**Configurazione:** Già attiva di default, nessuna modifica richiesta.

```bash
# Il sistema è accessibile solo da:
http://localhost:3000
```

---

## 🏢 Scenario 2: Accesso RETE LOCALE (LAN)

**Quando usarlo:** Accesso da altri dispositivi nella stessa rete (es: ufficio, casa).

### 📝 Step 1: Trova l'IP del server

Sul server dove gira Docker, esegui:

```bash
# Linux/Mac
ip addr show | grep "inet " | grep -v 127.0.0.1

# Windows
ipconfig | findstr IPv4
```

**Esempio output:** `192.168.1.100`

### 📝 Step 2: Modifica il file `.env`

Apri il file `.env` nella root del progetto:

```bash
cd /path/to/rag-enterprise
nano .env
```

Modifica la riga `VITE_API_URL`:

```bash
# Prima (localhost)
VITE_API_URL=http://localhost:8000

# Dopo (LAN) - sostituisci con il TUO IP
VITE_API_URL=http://192.168.1.100:8000
```

### 📝 Step 3: Ricostruisci e riavvia i container

```bash
cd rag-enterprise-structure

# Ricostruisci il frontend con la nuova configurazione
docker compose build frontend

# Riavvia tutti i servizi
docker compose up -d
```

### 📝 Step 4: Testa l'accesso

Da un altro dispositivo nella stessa rete, apri il browser e vai a:

```
http://192.168.1.100:3000
```

**✅ Fatto!** Ora puoi accedere da qualsiasi dispositivo nella tua rete locale.

---

### ⚠️ Firewall

Se non riesci ad accedere, potrebbe essere il firewall del server. Apri le porte necessarie:

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

## 🌍 Scenario 3: Accesso da INTERNET (Avanzato)

**Quando usarlo:** Accesso pubblico da qualsiasi luogo con connessione internet.

### ⚠️ Requisiti

1. **Dominio** (es: tuodominio.com) o Dynamic DNS
2. **IP pubblico statico** o Dynamic DNS service
3. **Reverse proxy** (Nginx) con SSL
4. **Port forwarding** sul router

### 🏗️ Architettura Consigliata

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

### 📝 Step 1: Configura Nginx Reverse Proxy

Installa Nginx sul server:

```bash
# Ubuntu/Debian
sudo apt update
sudo apt install nginx

# CentOS/RHEL
sudo yum install nginx
```

Crea il file di configurazione:

```bash
sudo nano /etc/nginx/sites-available/rag-enterprise
```

Incolla questa configurazione (sostituisci `tuodominio.com` con il tuo dominio):

```nginx
# HTTP (redirect to HTTPS)
server {
    listen 80;
    server_name tuodominio.com www.tuodominio.com;

    # Redirect to HTTPS
    return 301 https://$server_name$request_uri;
}

# HTTPS
server {
    listen 443 ssl http2;
    server_name tuodominio.com www.tuodominio.com;

    # SSL Certificate (usa Certbot per ottenerli gratuitamente)
    ssl_certificate /etc/letsencrypt/live/tuodominio.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/tuodominio.com/privkey.pem;

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

        # Timeout per LLM queries (può richiedere tempo)
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

    # Upload files (timeout maggiore)
    location /api/documents/upload {
        proxy_pass http://localhost:8000/api/documents/upload;
        proxy_http_version 1.1;
        client_max_body_size 100M;
        proxy_read_timeout 600s;
        proxy_connect_timeout 600s;
    }
}
```

Abilita il sito:

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

### 📝 Step 2: Ottieni Certificato SSL con Let's Encrypt

```bash
# Installa Certbot
sudo apt install certbot python3-certbot-nginx  # Ubuntu/Debian
# oppure
sudo yum install certbot python3-certbot-nginx  # CentOS/RHEL

# Ottieni certificato SSL gratuito
sudo certbot --nginx -d tuodominio.com -d www.tuodominio.com

# Certbot configurerà automaticamente Nginx con HTTPS
```

Il certificato si rinnoverà automaticamente. Verifica con:

```bash
sudo certbot renew --dry-run
```

---

### 📝 Step 3: Configura Port Forwarding sul Router

Accedi al pannello del tuo router (es: 192.168.1.1) e crea queste regole:

| Porta Esterna | Porta Interna | IP Interno       | Protocollo |
|---------------|---------------|------------------|------------|
| 80            | 80            | 192.168.1.100    | TCP        |
| 443           | 443           | 192.168.1.100    | TCP        |

*(Sostituisci 192.168.1.100 con l'IP del tuo server)*

---

### 📝 Step 4: Aggiorna `.env` per Internet

```bash
cd /path/to/rag-enterprise
nano .env
```

Modifica:

```bash
# Internet con SSL
VITE_API_URL=https://tuodominio.com/api
```

Ricostruisci:

```bash
cd rag-enterprise-structure
docker compose build frontend
docker compose up -d
```

---

### 📝 Step 5: Verifica Accesso

Da qualsiasi dispositivo connesso a internet, apri:

```
https://tuodominio.com
```

**✅ Fatto!** Il tuo sistema è accessibile da internet con SSL.

---

## 🔒 Sicurezza - Best Practices

### 1. **Cambia Password di Default**

Dopo il primo accesso, **cambia immediatamente** la password di default:

- Username: `admin`
- Password: `admin123`

Vai su: **🔑 Cambia Password** nell'interfaccia.

### 2. **Usa Password Forti**

- Minimo 12 caratteri
- Lettere maiuscole e minuscole
- Numeri e simboli

### 3. **Limita Accesso Admin**

Crea utenti con ruolo `user` o `super_user` per operazioni quotidiane.

### 4. **Backup Database**

Il database utenti è in:

```bash
rag-enterprise-structure/backend/data/rag_users.db
```

Fai backup regolari:

```bash
# Backup
cp rag-enterprise-structure/backend/data/rag_users.db ~/backup/rag_users_$(date +%Y%m%d).db

# Restore (se necessario)
cp ~/backup/rag_users_YYYYMMDD.db rag-enterprise-structure/backend/data/rag_users.db
docker compose restart backend
```

### 5. **Firewall Restrittivo**

Se usi LAN, blocca l'accesso da internet:

```bash
# Permetti solo dalla LAN
sudo ufw allow from 192.168.1.0/24 to any port 3000
sudo ufw allow from 192.168.1.0/24 to any port 8000
```

### 6. **Monitora Accessi**

Controlla i log:

```bash
# Log backend
docker compose logs backend --tail 100

# Log Nginx (se usato)
sudo tail -f /var/log/nginx/access.log
```

---

## 🐛 Troubleshooting

### Problema: "net::ERR_CONNECTION_REFUSED"

**Causa:** Frontend non riesce a contattare il backend.

**Soluzione:**
1. Verifica che `VITE_API_URL` nel `.env` sia corretto
2. Ricostruisci il frontend: `docker compose build frontend`
3. Verifica che il backend sia attivo: `docker compose ps`

### Problema: "401 Unauthorized" su tutte le richieste

**Causa:** Token JWT scaduto o non valido.

**Soluzione:**
1. Fai logout e login
2. Verifica che l'ora del server sia corretta: `date`

### Problema: CORS errors

**Causa:** Il backend blocca richieste da origini diverse.

**Soluzione:**
Il backend ha `allow_origins=["*"]` di default (vedi `app.py:161`). Se necessario, modifica:

```python
app.add_middleware(
    CORSMiddleware,
    allow_origins=["http://tuodominio.com", "https://tuodominio.com"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)
```

### Problema: Upload file fallisce

**Causa:** Timeout troppo breve o file troppo grande.

**Soluzione:**
1. Aumenta timeout Nginx (vedi configurazione sopra)
2. Aumenta `client_max_body_size` in Nginx

---

## 📞 Supporto

Per problemi o domande:
- Controlla i log: `docker compose logs backend --tail 100`
- Verifica health: `curl http://localhost:8000/health`
- GitHub Issues: (link al tuo repository)

---

## 🎯 Riepilogo Configurazioni

| Scenario | `.env` VITE_API_URL | Accessibile Da |
|----------|--------------------------|----------------|
| **Localhost** | `http://localhost:8000` | Solo questo computer |
| **LAN** | `http://192.168.1.100:8000` | Dispositivi nella rete locale |
| **Internet** | `https://tuodominio.com/api` | Ovunque con internet |

---

**Fatto!** 🎉 Il tuo sistema RAG Enterprise è configurato per l'accesso network.
