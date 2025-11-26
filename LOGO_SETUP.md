# 🎨 Guida Configurazione Logo Aziendale

Questa guida spiega come personalizzare il logo e il branding del sistema RAG Enterprise.

---

## 📋 Cosa Viene Visualizzato

Il logo appare in **3 posizioni**:

1. **Schermata di login** (centrato, 64px altezza)
2. **Header principale** (in alto a sinistra, 40px altezza)
3. **Footer** con "Powered by I3K Technologies Ltd."

---

## 🚀 Procedura Rapida

### Step 1: Prepara il Logo

**Requisiti:**
- **Formato**: PNG con sfondo trasparente (consigliato)
- **Dimensioni**: circa 200x60 pixel (proporzioni orizzontali)
- **Nome file**: `logo.png` (o qualsiasi nome, ma modifica il codice di conseguenza)

### Step 2: Carica il Logo

```bash
# Copia il tuo logo nella cartella public del frontend
cp /path/al/tuo/logo.png /home/user/rag-enterprise/frontend/public/logo.png
```

**Verifica che il file sia presente:**
```bash
ls -lh /home/user/rag-enterprise/frontend/public/logo.png
```

### Step 3: Il Logo è Già Configurato!

Il codice è già configurato per usare `/logo.png`. Non devi modificare nulla in `App.jsx`.

**Configurazione attuale:**
```javascript
const BRANDING = {
  clientLogo: '/logo.png',           // ← Punta al tuo logo
  clientName: 'RAG Enterprise',
  primaryColor: '#3b82f6',
  poweredBy: 'I3K Technologies',
  poweredBySubtitle: 'Ltd.',         // ← Aggiunto
  version: 'v1.1'
}
```

### Step 4: Ricostruisci il Frontend

```bash
cd /home/user/rag-enterprise/rag-enterprise-structure

# Ricostruisci il frontend per includere il logo
docker compose build frontend

# Riavvia
docker compose up -d frontend
```

### Step 5: Verifica

Apri il browser e vai su:
```
http://192.168.1.165:3000
# oppure
https://rag.i3k.eu
```

Il logo dovrebbe apparire:
- ✅ Nella schermata di login
- ✅ Nell'header dopo il login
- ✅ Footer con "I3K Technologies" e "Ltd." sotto

---

## 🎨 Personalizzazione Avanzata

### Cambiare Nome Logo

Se vuoi usare un nome diverso da `logo.png`:

1. Carica il file:
   ```bash
   cp /path/tuo-logo.svg /home/user/rag-enterprise/frontend/public/company-logo.svg
   ```

2. Modifica `App.jsx`:
   ```javascript
   const BRANDING = {
     clientLogo: '/company-logo.svg',  // ← Cambia qui
     // ...
   }
   ```

3. Ricostruisci: `docker compose build frontend`

### Cambiare Colore Primario

```javascript
const BRANDING = {
  // ...
  primaryColor: '#FF5733',  // ← Il tuo colore aziendale (hex)
}
```

### Rimuovere il Logo (Tornare al Testo)

```javascript
const BRANDING = {
  clientLogo: null,  // ← null = mostra testo invece del logo
  clientName: 'RAG Enterprise',
  // ...
}
```

### Cambiare "Powered by"

```javascript
const BRANDING = {
  // ...
  poweredBy: 'La Tua Azienda',
  poweredBySubtitle: 'S.r.l.',  // oppure null per rimuovere
}
```

---

## 📐 Dimensioni Logo Consigliate

### Logo Orizzontale (Consigliato)
- **Dimensioni**: 200x60 px
- **Proporzioni**: 3:1 o 4:1 (larghezza:altezza)
- **Formato**: PNG trasparente

### Logo Quadrato (Alternativa)
- **Dimensioni**: 120x120 px
- **Formato**: PNG trasparente

---

## 🔧 Troubleshooting

### Logo Non Appare

**Verifica 1:** Il file esiste?
```bash
ls /home/user/rag-enterprise/frontend/public/logo.png
```

**Verifica 2:** Hai ricostruito il frontend?
```bash
docker compose build frontend
docker compose up -d frontend
```

**Verifica 3:** Controlla i log del browser (F12):
- Se vedi errore 404 su `/logo.png`, il file non è stato copiato correttamente

### Logo Troppo Grande/Piccolo

Modifica le classi CSS in `App.jsx`:

**Login screen:**
```javascript
<img src={BRANDING.clientLogo} alt="Logo" className="h-16 mx-auto mb-4" />
//                                                       ↑
// Cambia h-16 in: h-12 (piccolo), h-20 (grande), h-24 (molto grande)
```

**Header:**
```javascript
<img src={BRANDING.clientLogo} alt="Logo" className="h-10" />
//                                                       ↑
// Cambia h-10 in: h-8 (piccolo), h-12 (grande)
```

### Logo Sfocato

Il tuo logo è troppo piccolo. Usa una risoluzione doppia:
- Se mostri il logo a 200px larghezza, usa un file da 400px

---

## 📝 Esempio Completo

```bash
# 1. Prepara logo (es: Photoshop, GIMP, Canva)
# - Dimensioni: 200x60 px
# - Formato: PNG trasparente
# - Salva come: logo-i3k.png

# 2. Carica sul server
scp logo-i3k.png user@server:/tmp/

# 3. Sul server, copia nella cartella corretta
cp /tmp/logo-i3k.png /home/user/rag-enterprise/frontend/public/logo.png

# 4. Ricostruisci frontend
cd /home/user/rag-enterprise/rag-enterprise-structure
docker compose build frontend
docker compose up -d frontend

# 5. Testa nel browser
# Vai su https://rag.i3k.eu e verifica il logo
```

---

## ✅ Checklist Finale

- [ ] Logo preparato (PNG trasparente, 200x60 px)
- [ ] Logo copiato in `/home/user/rag-enterprise/frontend/public/logo.png`
- [ ] Frontend ricostruito: `docker compose build frontend`
- [ ] Container riavviato: `docker compose up -d frontend`
- [ ] Testato nel browser
- [ ] Logo visibile su login e header
- [ ] Footer mostra "I3K Technologies Ltd."

---

## 🎯 Risultato

Dopo aver seguito questa guida, il tuo sistema RAG avrà:

✅ Logo aziendale personalizzato
✅ Branding coerente su tutte le schermate
✅ Footer con "I3K Technologies Ltd."
✅ Aspetto professionale e brandizzato

**Fatto!** 🎉
