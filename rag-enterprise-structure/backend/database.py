"""
Database setup e modelli per sistema autenticazione
"""

import sqlite3
from datetime import datetime
from typing import Optional, List, Dict
import bcrypt
import logging

logger = logging.getLogger(__name__)

DB_PATH = "rag_users.db"


class UserRole:
    """Definizione ruoli utente"""
    ADMIN = "admin"
    SUPER_USER = "super_user"
    USER = "user"

    @classmethod
    def all_roles(cls):
        return [cls.ADMIN, cls.SUPER_USER, cls.USER]


class UserDatabase:
    """Gestione database utenti"""

    def __init__(self, db_path: str = DB_PATH):
        self.db_path = db_path
        self.init_db()

    def get_connection(self):
        """Crea connessione al database"""
        conn = sqlite3.connect(self.db_path)
        conn.row_factory = sqlite3.Row  # Per accedere alle colonne per nome
        return conn

    def init_db(self):
        """Inizializza database e crea utente admin di default"""
        conn = self.get_connection()
        cursor = conn.cursor()

        # Crea tabella utenti
        cursor.execute('''
            CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT UNIQUE NOT NULL,
                email TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                role TEXT NOT NULL,
                created_at TEXT NOT NULL,
                last_login TEXT,
                is_active INTEGER DEFAULT 1
            )
        ''')

        conn.commit()

        # Crea utente admin di default se non esiste
        try:
            admin_exists = cursor.execute(
                "SELECT id FROM users WHERE username = ?",
                ("admin",)
            ).fetchone()

            if not admin_exists:
                self.create_user(
                    username="admin",
                    email="admin@rag-enterprise.local",
                    password="admin123",  # Password di default - DA CAMBIARE!
                    role=UserRole.ADMIN
                )
                logger.info("✅ Utente admin creato (username: admin, password: admin123)")
                logger.warning("⚠️  CAMBIA LA PASSWORD ADMIN!")
        except Exception as e:
            logger.error(f"Errore creazione admin: {e}")

        conn.close()

    def hash_password(self, password: str) -> str:
        """Hash della password con bcrypt"""
        salt = bcrypt.gensalt()
        return bcrypt.hashpw(password.encode('utf-8'), salt).decode('utf-8')

    def verify_password(self, password: str, password_hash: str) -> bool:
        """Verifica password contro hash"""
        return bcrypt.checkpw(
            password.encode('utf-8'),
            password_hash.encode('utf-8')
        )

    def create_user(
        self,
        username: str,
        email: str,
        password: str,
        role: str
    ) -> Optional[int]:
        """Crea nuovo utente"""
        if role not in UserRole.all_roles():
            raise ValueError(f"Ruolo non valido: {role}")

        conn = self.get_connection()
        cursor = conn.cursor()

        try:
            password_hash = self.hash_password(password)
            cursor.execute(
                '''INSERT INTO users
                   (username, email, password_hash, role, created_at)
                   VALUES (?, ?, ?, ?, ?)''',
                (username, email, password_hash, role, datetime.utcnow().isoformat())
            )
            conn.commit()
            user_id = cursor.lastrowid
            logger.info(f"✅ Utente creato: {username} (role: {role})")
            return user_id
        except sqlite3.IntegrityError as e:
            logger.error(f"❌ Errore creazione utente: {e}")
            return None
        finally:
            conn.close()

    def get_user_by_username(self, username: str) -> Optional[Dict]:
        """Recupera utente per username"""
        conn = self.get_connection()
        cursor = conn.cursor()

        row = cursor.execute(
            "SELECT * FROM users WHERE username = ? AND is_active = 1",
            (username,)
        ).fetchone()

        conn.close()

        if row:
            return dict(row)
        return None

    def get_user_by_id(self, user_id: int) -> Optional[Dict]:
        """Recupera utente per ID"""
        conn = self.get_connection()
        cursor = conn.cursor()

        row = cursor.execute(
            "SELECT * FROM users WHERE id = ? AND is_active = 1",
            (user_id,)
        ).fetchone()

        conn.close()

        if row:
            return dict(row)
        return None

    def authenticate_user(self, username: str, password: str) -> Optional[Dict]:
        """Autentica utente"""
        user = self.get_user_by_username(username)

        if not user:
            return None

        if not self.verify_password(password, user['password_hash']):
            return None

        # Aggiorna last_login
        self.update_last_login(user['id'])

        return user

    def update_last_login(self, user_id: int):
        """Aggiorna timestamp ultimo login"""
        conn = self.get_connection()
        cursor = conn.cursor()

        cursor.execute(
            "UPDATE users SET last_login = ? WHERE id = ?",
            (datetime.utcnow().isoformat(), user_id)
        )
        conn.commit()
        conn.close()

    def list_users(self) -> List[Dict]:
        """Lista tutti gli utenti attivi"""
        conn = self.get_connection()
        cursor = conn.cursor()

        rows = cursor.execute(
            "SELECT id, username, email, role, created_at, last_login FROM users WHERE is_active = 1"
        ).fetchall()

        conn.close()

        return [dict(row) for row in rows]

    def update_user_role(self, user_id: int, new_role: str) -> bool:
        """Aggiorna ruolo utente"""
        if new_role not in UserRole.all_roles():
            raise ValueError(f"Ruolo non valido: {new_role}")

        conn = self.get_connection()
        cursor = conn.cursor()

        cursor.execute(
            "UPDATE users SET role = ? WHERE id = ?",
            (new_role, user_id)
        )
        conn.commit()
        affected = cursor.rowcount
        conn.close()

        return affected > 0

    def delete_user(self, user_id: int) -> bool:
        """Disabilita utente (soft delete)"""
        conn = self.get_connection()
        cursor = conn.cursor()

        cursor.execute(
            "UPDATE users SET is_active = 0 WHERE id = ?",
            (user_id,)
        )
        conn.commit()
        affected = cursor.rowcount
        conn.close()

        return affected > 0

    def change_password(self, user_id: int, new_password: str) -> bool:
        """Cambia password utente"""
        password_hash = self.hash_password(new_password)

        conn = self.get_connection()
        cursor = conn.cursor()

        cursor.execute(
            "UPDATE users SET password_hash = ? WHERE id = ?",
            (password_hash, user_id)
        )
        conn.commit()
        affected = cursor.rowcount
        conn.close()

        return affected > 0


# Istanza globale
db = UserDatabase()
