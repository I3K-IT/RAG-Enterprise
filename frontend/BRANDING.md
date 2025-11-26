# Frontend Branding Customization

The RAG Enterprise frontend can be customized for each client by modifying the `BRANDING` object in `src/App.jsx`.

## Branding Configuration

```javascript
const BRANDING = {
  clientLogo: null,              // Client logo URL
  clientName: 'RAG Enterprise',  // Name to display if logo = null
  primaryColor: '#3b82f6',       // Primary color (Tailwind blue-500)
  poweredBy: 'I3K Technologies', // Developer name (footer)
  version: 'v1.0'                // System version
}
```

## Logo Customization

### Option 1: Image logo

Insert the client logo in the `public/` folder and modify:

```javascript
const BRANDING = {
  clientLogo: '/client-logo.png',  // Path relative to /public
  clientName: 'Client Company',
  // ...
}
```

### Option 2: Text only

Leave `clientLogo: null` and the system will use `clientName` as text:

```javascript
const BRANDING = {
  clientLogo: null,
  clientName: 'Client Company Ltd.',
  // ...
}
```

## Color Customization

The primary color can be modified but must be a valid Tailwind CSS color.

**Recommended colors:**
- `#3b82f6` (blue-500) - Default
- `#10b981` (green-500) - Green
- `#8b5cf6` (violet-500) - Violet
- `#f59e0b` (amber-500) - Orange
- `#ef4444` (red-500) - Red

## Footer

The footer always shows:
- **"Powered by I3K Technologies"** (not modifiable, development contract)
- Clickable disclaimer
- Privacy link
- System version

The disclaimer automatically includes I3K Technologies copyright.

## Complete Example

```javascript
const BRANDING = {
  clientLogo: '/logo-acme.png',
  clientName: 'ACME Corporation',
  primaryColor: '#10b981',  // Company green
  poweredBy: 'I3K Technologies',
  version: 'v1.0'
}
```

## Build and Deploy

After modifying branding:

```bash
cd frontend
npm run build
```

The frontend will be rebuilt with the new branding.

## Legal Notes

The footer **MUST always include** "Powered by I3K Technologies" as per software development contract.

Removal of this notice constitutes a violation of the license agreement.
