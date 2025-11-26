# 🎨 Company Logo Configuration Guide

This guide explains how to customize the logo and branding of the RAG Enterprise system.

---

## 📋 What Is Displayed

The logo appears in **3 positions**:

1. **Login screen** (centered, 64px height)
2. **Main header** (top left, 40px height)
3. **Footer** with "Powered by I3K Technologies Ltd."

---

## 🚀 Quick Procedure

### Step 1: Prepare the Logo

**Requirements:**
- **Format**: PNG with transparent background (recommended)
- **Dimensions**: approximately 200x60 pixels (horizontal proportions)
- **File name**: `logo.png` (or any name, but modify the code accordingly)

### Step 2: Upload the Logo

```bash
# Copy your logo to the frontend public folder
cp /path/to/your/logo.png /home/user/rag-enterprise/frontend/public/logo.png
```

**Verify that the file is present:**
```bash
ls -lh /home/user/rag-enterprise/frontend/public/logo.png
```

### Step 3: The Logo Is Already Configured!

The code is already configured to use `/logo.png`. You don't need to modify anything in `App.jsx`.

**Current configuration:**
```javascript
const BRANDING = {
  clientLogo: '/logo.png',           // ← Points to your logo
  clientName: 'RAG Enterprise',
  primaryColor: '#3b82f6',
  poweredBy: 'I3K Technologies',
  poweredBySubtitle: 'Ltd.',         // ← Added
  version: 'v1.1'
}
```

### Step 4: Rebuild the Frontend

```bash
cd /home/user/rag-enterprise/rag-enterprise-structure

# Rebuild the frontend to include the logo
docker compose build frontend

# Restart
docker compose up -d frontend
```

### Step 5: Verify

Open the browser and go to:
```
http://192.168.1.165:3000
# or
https://rag.i3k.eu
```

The logo should appear:
- ✅ In the login screen
- ✅ In the header after login
- ✅ Footer with "I3K Technologies" and "Ltd." below

---

## 🎨 Advanced Customization

### Change Logo Name

If you want to use a different name than `logo.png`:

1. Upload the file:
   ```bash
   cp /path/your-logo.svg /home/user/rag-enterprise/frontend/public/company-logo.svg
   ```

2. Modify `App.jsx`:
   ```javascript
   const BRANDING = {
     clientLogo: '/company-logo.svg',  // ← Change here
     // ...
   }
   ```

3. Rebuild: `docker compose build frontend`

### Change Primary Color

```javascript
const BRANDING = {
  // ...
  primaryColor: '#FF5733',  // ← Your company color (hex)
}
```

### Remove the Logo (Return to Text)

```javascript
const BRANDING = {
  clientLogo: null,  // ← null = show text instead of logo
  clientName: 'RAG Enterprise',
  // ...
}
```

### Change "Powered by"

```javascript
const BRANDING = {
  // ...
  poweredBy: 'Your Company',
  poweredBySubtitle: 'Ltd.',  // or null to remove
}
```

---

## 📐 Recommended Logo Dimensions

### Horizontal Logo (Recommended)
- **Dimensions**: 200x60 px
- **Proportions**: 3:1 or 4:1 (width:height)
- **Format**: Transparent PNG

### Square Logo (Alternative)
- **Dimensions**: 120x120 px
- **Format**: Transparent PNG

---

## 🔧 Troubleshooting

### Logo Does Not Appear

**Check 1:** Does the file exist?
```bash
ls /home/user/rag-enterprise/frontend/public/logo.png
```

**Check 2:** Did you rebuild the frontend?
```bash
docker compose build frontend
docker compose up -d frontend
```

**Check 3:** Check browser logs (F12):
- If you see 404 error on `/logo.png`, the file was not copied correctly

### Logo Too Large/Small

Modify the CSS classes in `App.jsx`:

**Login screen:**
```javascript
<img src={BRANDING.clientLogo} alt="Logo" className="h-16 mx-auto mb-4" />
//                                                       ↑
// Change h-16 to: h-12 (small), h-20 (large), h-24 (very large)
```

**Header:**
```javascript
<img src={BRANDING.clientLogo} alt="Logo" className="h-10" />
//                                                       ↑
// Change h-10 to: h-8 (small), h-12 (large)
```

### Blurry Logo

Your logo is too small. Use double resolution:
- If showing logo at 200px width, use a 400px file

---

## 📝 Complete Example

```bash
# 1. Prepare logo (e.g., Photoshop, GIMP, Canva)
# - Dimensions: 200x60 px
# - Format: Transparent PNG
# - Save as: logo-i3k.png

# 2. Upload to server
scp logo-i3k.png user@server:/tmp/

# 3. On the server, copy to correct folder
cp /tmp/logo-i3k.png /home/user/rag-enterprise/frontend/public/logo.png

# 4. Rebuild frontend
cd /home/user/rag-enterprise/rag-enterprise-structure
docker compose build frontend
docker compose up -d frontend

# 5. Test in browser
# Go to https://rag.i3k.eu and verify the logo
```

---

## ✅ Final Checklist

- [ ] Logo prepared (transparent PNG, 200x60 px)
- [ ] Logo copied to `/home/user/rag-enterprise/frontend/public/logo.png`
- [ ] Frontend rebuilt: `docker compose build frontend`
- [ ] Container restarted: `docker compose up -d frontend`
- [ ] Tested in browser
- [ ] Logo visible on login and header
- [ ] Footer shows "I3K Technologies Ltd."

---

## 🎯 Result

After following this guide, your RAG system will have:

✅ Customized company logo
✅ Consistent branding across all screens
✅ Footer with "I3K Technologies Ltd."
✅ Professional and branded appearance

**Done!** 🎉
