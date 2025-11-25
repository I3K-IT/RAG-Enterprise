# Personalizzazione Branding Frontend

Il frontend RAG Enterprise può essere personalizzato per ogni cliente modificando l'oggetto `BRANDING` in `src/App.jsx`.

## Configurazione Branding

```javascript
const BRANDING = {
  clientLogo: null,              // URL del logo cliente
  clientName: 'RAG Enterprise',  // Nome da mostrare se logo = null
  primaryColor: '#3b82f6',       // Colore primario (Tailwind blue-500)
  poweredBy: 'I3K Technologies', // Nome realizzatore (footer)
  version: 'v1.0'                // Versione sistema
}
```

## Personalizzazione Logo

### Opzione 1: Logo immagine

Inserisci il logo del cliente nella cartella `public/` e modifica:

```javascript
const BRANDING = {
  clientLogo: '/logo-cliente.png',  // Path relativo a /public
  clientName: 'Azienda Cliente',
  // ...
}
```

### Opzione 2: Solo testo

Lascia `clientLogo: null` e il sistema userà `clientName` come testo:

```javascript
const BRANDING = {
  clientLogo: null,
  clientName: 'Azienda Cliente S.r.l.',
  // ...
}
```

## Personalizzazione Colori

Il colore primario può essere modificato ma deve essere un colore Tailwind CSS valido.

**Colori consigliati:**
- `#3b82f6` (blue-500) - Default
- `#10b981` (green-500) - Verde
- `#8b5cf6` (violet-500) - Viola
- `#f59e0b` (amber-500) - Arancione
- `#ef4444` (red-500) - Rosso

## Footer

Il footer mostra sempre:
- **"Powered by I3K Technologies"** (non modificabile, contratto sviluppo)
- Disclaimer cliccabile
- Link Privacy
- Versione sistema

Il disclaimer include automaticamente il copyright I3K Technologies.

## Esempio Completo

```javascript
const BRANDING = {
  clientLogo: '/logo-acme.png',
  clientName: 'ACME Corporation',
  primaryColor: '#10b981',  // Verde aziendale
  poweredBy: 'I3K Technologies',
  version: 'v1.0'
}
```

## Build e Deploy

Dopo aver modificato il branding:

```bash
cd frontend
npm run build
```

Il frontend verrà ricostruito con il nuovo branding.

## Note Legali

Il footer **DEVE sempre includere** "Powered by I3K Technologies" come da contratto di sviluppo software.

La rimozione di questa dicitura costituisce violazione del contratto di licenza.
