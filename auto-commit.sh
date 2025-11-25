#!/bin/bash

# Usa la directory dove si trova lo script
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR"

# Controlla se ci sono modifiche
if [ -z "$(git status --porcelain)" ]; then
    echo "✓ Nessuna modifica"
    exit 0
fi

# Mostra cosa è cambiato
echo "📝 Modifiche rilevate:"
git status --short

# Chiedi conferma
read -p "Committo e pusho? (y/n) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    exit 0
fi

# Aggiungi tutto
git add .

# Chiedi messaggio commit
read -p "Messaggio commit: " msg
if [ -z "$msg" ]; then
    msg="Update: $(date +%Y-%m-%d\ %H:%M)"
fi

# Commit e push
git commit -m "$msg"
git push

echo "✅ Committed e pushed!"
