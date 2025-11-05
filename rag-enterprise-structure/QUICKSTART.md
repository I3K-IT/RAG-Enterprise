# Quick Start Guide

## Scenario 1: Installazione Completa (Monolith)

Se vuoi tutto su una singola macchina:

```bash
chmod +x setup.sh
./setup.sh
```

Poi accedi a: **http://localhost:3000**

---

## Scenario 2: Installazione Distribuita

### Macchina 1: Database + Backend

```bash
chmod +x setup-backend-only.sh
./setup-backend-only.sh
```

La macchina sarà disponibile all'indirizzo: `http://<IP_MACCHINA_1>:8000`

### Macchina 2: Frontend

Modifica `.env`:
```bash
BACKEND_URL=http://<IP_MACCHINA_1>:8000
```

Poi:
```bash
chmod +x setup-frontend-only.sh
./setup-frontend-only.sh
```

Accedi a: **http://localhost:3000**

---

## Scenario 3: Multi-Location (Remote)

### Datacenter 1: Database Separato

```bash
chmod +x setup-database-only.sh
./setup-database-only.sh
```

Database disponibile su: `milvius.datacenter.com:19530`

### Datacenter 2: Backend

Modifica `.env`:
```bash
MILVIUS_HOST=milvius.datacenter.com
MILVIUS_PORT=19530
```

```bash
chmod +x setup-backend-only.sh
./setup-backend-only.sh
```

### Ufficio/Locale: Frontend

Modifica `.env`:
```bash
BACKEND_URL=http://backend.datacenter.com:8000
```

```bash
chmod +x setup-frontend-only.sh
./setup-frontend-only.sh
```

---

## Comandi Utili

```bash
# Visualizza log
make logs

# Arresta servizi
make stop

# Restart
make restart

# Verifica salute
make health

# Pulizia completa
make clean
```

---

## Troubleshooting

### Backend non si connette a Milvius

1. Verifica MILVIUS_HOST e MILVIUS_PORT in `.env`
2. Verifica firewall
3. Test manuale: `telnet milvius.host 19530`

### Frontend mostra errore di connessione

1. Verifica BACKEND_URL in `.env`
2. Verifica backend è online: `curl http://backend:8000/health`
3. Controlla CORS se backend e frontend in domini diversi

### GPU non riconosciuta

1. Installa NVIDIA Docker: `https://github.com/NVIDIA/nvidia-docker`
2. Test: `docker run --rm --gpus all nvidia/cuda:12.0.0-runtime nvidia-smi`

### Modelli LLM non caricano

I modelli si scaricano automaticamente la prima volta. Dipende da:
- Connessione internet (modelli sono GB)
- Spazio disco

Aspetta il completamento nel log: `docker-compose logs ollama`

---

## Performance Tips

### Se sistema è lento:

1. Aumenta VRAM: modifica `CUDA_VISIBLE_DEVICES` per multipli GPU
2. Riduci `EMBEDDING_BATCH_SIZE` in `.env` se OOM
3. Usa modello LLM più piccolo: `NEURAL_CHAT_7B` vs `MISTRAL_LARGE`

### Se vuoi velocità massima:

```bash
# .env
EMBEDDING_MODEL=all-MiniLM-L6-v2
LLM_MODEL=neural-chat
EMBEDDING_BATCH_SIZE=64
```

---

## Backup

### Salva dati Milvius

```bash
docker-compose exec milvius /bin/bash -c "tar -czf /tmp/milvius-backup.tar.gz /var/lib/milvius"
docker cp rag-milvius:/tmp/milvius-backup.tar.gz ./milvius-backup.tar.gz
```

### Restore

```bash
docker cp ./milvius-backup.tar.gz rag-milvius:/tmp/
docker-compose exec milvius /bin/bash -c "cd / && tar -xzf /tmp/milvius-backup.tar.gz"
```

---

## Supporto

Per problemi, controlla:

1. `docker-compose logs`
2. `docker-compose ps` (verifica che tutti siano running)
3. Health endpoints:
   - Backend: `curl http://localhost:8000/health`
   - Frontend: `curl http://localhost:3000`
