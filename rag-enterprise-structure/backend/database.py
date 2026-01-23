"""
Database setup and models for authentication system
"""

import sqlite3
from datetime import datetime
from typing import Optional, List, Dict
import bcrypt
import logging
import os

logger = logging.getLogger(__name__)

# Database path - use /app/data in Docker, ./data locally
DB_PATH = os.getenv("DB_PATH", "/app/data/rag_users.db")


class UserRole:
    """User role definitions"""
    ADMIN = "admin"
    SUPER_USER = "super_user"
    USER = "user"

    @classmethod
    def all_roles(cls):
        return [cls.ADMIN, cls.SUPER_USER, cls.USER]


class UserDatabase:
    """User database management"""

    def __init__(self, db_path: str = DB_PATH):
        self.db_path = db_path
        # Create directory if it doesn't exist
        db_dir = os.path.dirname(db_path)
        if db_dir and not os.path.exists(db_dir):
            os.makedirs(db_dir, exist_ok=True)
        self.init_db()

    def get_connection(self):
        """Create database connection"""
        conn = sqlite3.connect(self.db_path)
        conn.row_factory = sqlite3.Row  # To access columns by name
        return conn

    def init_db(self):
        """Initialize database and create default admin user"""
        conn = self.get_connection()
        cursor = conn.cursor()

        # Create users table
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

        # Create default admin user if it doesn't exist
        try:
            admin_exists = cursor.execute(
                "SELECT id FROM users WHERE username = ?",
                ("admin",)
            ).fetchone()

            if not admin_exists:
                # Get admin password from environment or generate a secure random one
                import secrets
                default_admin_password = os.getenv("ADMIN_DEFAULT_PASSWORD", "")

                if not default_admin_password:
                    # Generate a secure random password
                    default_admin_password = secrets.token_urlsafe(16)
                    logger.warning("=" * 70)
                    logger.warning("🔐 ADMIN ACCOUNT CREATED WITH RANDOM PASSWORD")
                    logger.warning("")
                    logger.warning(f"   Username: admin")
                    logger.warning(f"   Password: {default_admin_password}")
                    logger.warning("")
                    logger.warning("⚠️  SAVE THIS PASSWORD NOW - it won't be shown again!")
                    logger.warning("You can change it after login in the admin panel.")
                    logger.warning("")
                    logger.warning("To set a specific password, add to .env:")
                    logger.warning("   ADMIN_DEFAULT_PASSWORD=your-secure-password")
                    logger.warning("=" * 70)
                else:
                    logger.info("✅ Admin user created with password from ADMIN_DEFAULT_PASSWORD")

                self.create_user(
                    username="admin",
                    email="admin@rag-enterprise.local",
                    password=default_admin_password,
                    role=UserRole.ADMIN
                )
        except Exception as e:
            logger.error(f"Error creating admin: {e}")

        conn.close()

    def hash_password(self, password: str) -> str:
        """Hash password with bcrypt"""
        salt = bcrypt.gensalt()
        return bcrypt.hashpw(password.encode('utf-8'), salt).decode('utf-8')

    def verify_password(self, password: str, password_hash: str) -> bool:
        """Verify password against hash"""
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
        """Create new user"""
        if role not in UserRole.all_roles():
            raise ValueError(f"Invalid role: {role}")

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
            logger.info(f"✅ User created: {username} (role: {role})")
            return user_id
        except sqlite3.IntegrityError as e:
            logger.error(f"❌ User creation error: {e}")
            return None
        finally:
            conn.close()

    def get_user_by_username(self, username: str) -> Optional[Dict]:
        """Retrieve user by username"""
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
        """Retrieve user by ID"""
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
        """Authenticate user"""
        user = self.get_user_by_username(username)

        if not user:
            return None

        if not self.verify_password(password, user['password_hash']):
            return None

        # Update last_login
        self.update_last_login(user['id'])

        return user

    def update_last_login(self, user_id: int):
        """Update last login timestamp"""
        conn = self.get_connection()
        cursor = conn.cursor()

        cursor.execute(
            "UPDATE users SET last_login = ? WHERE id = ?",
            (datetime.utcnow().isoformat(), user_id)
        )
        conn.commit()
        conn.close()

    def list_users(self) -> List[Dict]:
        """List all active users"""
        conn = self.get_connection()
        cursor = conn.cursor()

        rows = cursor.execute(
            "SELECT id, username, email, role, created_at, last_login FROM users WHERE is_active = 1"
        ).fetchall()

        conn.close()

        return [dict(row) for row in rows]

    def update_user_role(self, user_id: int, new_role: str) -> bool:
        """Update user role"""
        if new_role not in UserRole.all_roles():
            raise ValueError(f"Invalid role: {new_role}")

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
        """Disable user (soft delete)"""
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
        """Change user password"""
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


# Global instance
db = UserDatabase()
