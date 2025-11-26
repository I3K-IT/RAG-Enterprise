import React, { useState, useEffect, useRef } from 'react'
import axios from 'axios'
import './App.css'

// Configurazione logo e branding (personalizzabile per ogni cliente)
const BRANDING = {
  clientLogo: '/logo.png', // URL del logo cliente, null = usa nome testuale
  clientName: 'RAG Enterprise',
  primaryColor: '#3b82f6', // blue-500
  poweredBy: 'I3K Technologies',
  poweredBySubtitle: 'Ltd.',
  version: 'v1.1'
}

// API Configuration - usa variabile d'ambiente o fallback a localhost
const API_URL = import.meta.env.VITE_API_URL || 'http://localhost:8000'

function App() {
  // Authentication state
  const [isAuthenticated, setIsAuthenticated] = useState(false)
  const [user, setUser] = useState(null)
  const [token, setToken] = useState(null)
  const [loginForm, setLoginForm] = useState({ username: '', password: '' })
  const [loggingIn, setLoggingIn] = useState(false)
  const [loginError, setLoginError] = useState('')

  // Admin panel state
  const [showAdminPanel, setShowAdminPanel] = useState(false)
  const [allUsers, setAllUsers] = useState([])
  const [loadingUsers, setLoadingUsers] = useState(false)
  const [newUserForm, setNewUserForm] = useState({
    username: '',
    email: '',
    password: '',
    role: 'user'
  })
  const [creatingUser, setCreatingUser] = useState(false)

  // Change password state
  const [showChangePasswordModal, setShowChangePasswordModal] = useState(false)
  const [passwordForm, setPasswordForm] = useState({
    oldPassword: '',
    newPassword: '',
    confirmPassword: ''
  })
  const [changingPassword, setChangingPassword] = useState(false)
  const [passwordError, setPasswordError] = useState('')

  // Backend status
  const [status, setStatus] = useState('checking')
  const [backendHealth, setBackendHealth] = useState(null)

  // Conversazioni
  const [conversations, setConversations] = useState([])
  const [currentConversationId, setCurrentConversationId] = useState(null)
  const [messages, setMessages] = useState([])

  // Input query
  const [query, setQuery] = useState('')
  const [querying, setQuerying] = useState(false)
  const [isModelLoading, setIsModelLoading] = useState(false)

  // Documenti
  const [documents, setDocuments] = useState([])
  const [loadingDocuments, setLoadingDocuments] = useState(false)
  const [uploadProgress, setUploadProgress] = useState(0)
  const [uploading, setUploading] = useState(false)
  const [uploadPhase, setUploadPhase] = useState('')

  // UI state
  const [showConversationsSidebar, setShowConversationsSidebar] = useState(true)
  const [showDocumentsSidebar, setShowDocumentsSidebar] = useState(true)

  const messagesEndRef = useRef(null)
  const fileInputRef = useRef(null)
  const modelLoadingTimerRef = useRef(null)

  // Scroll to bottom when new messages arrive
  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }

  useEffect(scrollToBottom, [messages])

  // Initialize: check token and load data
  useEffect(() => {
    const savedToken = localStorage.getItem('rag_auth_token')
    const savedUser = localStorage.getItem('rag_auth_user')

    if (savedToken && savedUser) {
      setToken(savedToken)
      setUser(JSON.parse(savedUser))
      setIsAuthenticated(true)
    }
  }, [])

  // When authenticated, load conversations and check backend
  useEffect(() => {
    if (isAuthenticated) {
      loadConversationsFromStorage()
      checkBackendHealth()
      fetchDocuments()

      const interval = setInterval(checkBackendHealth, 30000)
      return () => clearInterval(interval)
    }
  }, [isAuthenticated])

  // Axios interceptor to add Authorization header
  useEffect(() => {
    const requestInterceptor = axios.interceptors.request.use(
      (config) => {
        if (token) {
          config.headers.Authorization = `Bearer ${token}`
        }
        return config
      },
      (error) => Promise.reject(error)
    )

    const responseInterceptor = axios.interceptors.response.use(
      (response) => response,
      (error) => {
        // Logout if 401 Unauthorized
        if (error.response?.status === 401) {
          handleLogout()
        }
        return Promise.reject(error)
      }
    )

    return () => {
      axios.interceptors.request.eject(requestInterceptor)
      axios.interceptors.response.eject(responseInterceptor)
    }
  }, [token])

  // ============================================================================
  // AUTHENTICATION FUNCTIONS
  // ============================================================================

  const handleLogin = async (e) => {
    e.preventDefault()
    setLoggingIn(true)
    setLoginError('')

    try {
      const response = await axios.post(`${API_URL}/api/auth/login`, {
        username: loginForm.username,
        password: loginForm.password
      })

      const { access_token, user: userData } = response.data

      setToken(access_token)
      setUser(userData)
      setIsAuthenticated(true)

      // Save to localStorage
      localStorage.setItem('rag_auth_token', access_token)
      localStorage.setItem('rag_auth_user', JSON.stringify(userData))

      console.log('✅ Login successful:', userData.username, 'role:', userData.role)
    } catch (error) {
      console.error('Login error:', error)
      setLoginError(error.response?.data?.detail || 'Errore login')
    } finally {
      setLoggingIn(false)
    }
  }

  const handleLogout = () => {
    setToken(null)
    setUser(null)
    setIsAuthenticated(false)
    localStorage.removeItem('rag_auth_token')
    localStorage.removeItem('rag_auth_user')
    setLoginForm({ username: '', password: '' })
    setShowAdminPanel(false)
  }

  // ============================================================================
  // ADMIN FUNCTIONS
  // ============================================================================

  const fetchAllUsers = async () => {
    if (!user || user.role !== 'admin') return

    setLoadingUsers(true)
    try {
      const response = await axios.get(`${API_URL}/api/auth/users`)
      setAllUsers(response.data.users || [])
    } catch (error) {
      console.error('Error fetching users:', error)
      alert('Errore caricamento utenti')
    } finally {
      setLoadingUsers(false)
    }
  }

  const handleCreateUser = async (e) => {
    e.preventDefault()
    setCreatingUser(true)

    try {
      await axios.post(`${API_URL}/api/auth/users`, newUserForm)
      alert(`✅ Utente "${newUserForm.username}" creato con successo!`)
      setNewUserForm({ username: '', email: '', password: '', role: 'user' })
      fetchAllUsers()
    } catch (error) {
      console.error('Error creating user:', error)
      alert(`❌ Errore: ${error.response?.data?.detail || error.message}`)
    } finally {
      setCreatingUser(false)
    }
  }

  const handleDeleteUser = async (userId, username) => {
    if (!window.confirm(`Sei sicuro di voler eliminare l'utente "${username}"?`)) {
      return
    }

    try {
      await axios.delete(`${API_URL}/api/auth/users/${userId}`)
      alert('✅ Utente eliminato')
      fetchAllUsers()
    } catch (error) {
      console.error('Error deleting user:', error)
      alert(`❌ Errore: ${error.response?.data?.detail || error.message}`)
    }
  }

  const handleChangeUserRole = async (userId, newRole, username) => {
    try {
      await axios.put(`${API_URL}/api/auth/users/${userId}`, { role: newRole })
      alert(`✅ Ruolo di "${username}" aggiornato a "${newRole}"`)
      fetchAllUsers()
    } catch (error) {
      console.error('Error updating role:', error)
      alert(`❌ Errore: ${error.response?.data?.detail || error.message}`)
    }
  }

  const toggleAdminPanel = () => {
    const newState = !showAdminPanel
    setShowAdminPanel(newState)
    if (newState) {
      fetchAllUsers()
    }
  }

  // ============================================================================
  // PASSWORD CHANGE FUNCTIONS
  // ============================================================================

  const handleChangePassword = async (e) => {
    e.preventDefault()
    setPasswordError('')

    // Validate new password matches confirmation
    if (passwordForm.newPassword !== passwordForm.confirmPassword) {
      setPasswordError('Le nuove password non corrispondono')
      return
    }

    // Validate password length
    if (passwordForm.newPassword.length < 6) {
      setPasswordError('La nuova password deve essere di almeno 6 caratteri')
      return
    }

    setChangingPassword(true)

    try {
      await axios.post(`${API_URL}/api/auth/change-password`, {
        old_password: passwordForm.oldPassword,
        new_password: passwordForm.newPassword
      })

      alert('✅ Password cambiata con successo!')
      setShowChangePasswordModal(false)
      setPasswordForm({ oldPassword: '', newPassword: '', confirmPassword: '' })
    } catch (error) {
      console.error('Error changing password:', error)
      setPasswordError(error.response?.data?.detail || 'Errore cambio password')
    } finally {
      setChangingPassword(false)
    }
  }

  const toggleChangePasswordModal = () => {
    setShowChangePasswordModal(!showChangePasswordModal)
    setPasswordForm({ oldPassword: '', newPassword: '', confirmPassword: '' })
    setPasswordError('')
  }

  // ============================================================================
  // CONVERSATIONS FUNCTIONS
  // ============================================================================

  const loadConversationsFromStorage = () => {
    try {
      const stored = localStorage.getItem('rag_conversations')
      if (stored) {
        const parsed = JSON.parse(stored)
        setConversations(parsed)

        const lastActiveId = localStorage.getItem('rag_current_conversation')
        if (lastActiveId && parsed.find(c => c.id === lastActiveId)) {
          loadConversation(lastActiveId)
        } else if (parsed.length > 0) {
          loadConversation(parsed[0].id)
        }
      } else {
        createNewConversation()
      }
    } catch (error) {
      console.error('Errore caricamento conversazioni:', error)
      createNewConversation()
    }
  }

  const saveConversationsToStorage = (convs) => {
    localStorage.setItem('rag_conversations', JSON.stringify(convs))
  }

  const createNewConversation = () => {
    const newConv = {
      id: Date.now().toString(),
      title: 'Nuova Conversazione',
      messages: [],
      createdAt: new Date().toISOString()
    }

    const updated = [newConv, ...conversations]
    setConversations(updated)
    saveConversationsToStorage(updated)
    loadConversation(newConv.id)
  }

  const loadConversation = (convId) => {
    const conv = conversations.find(c => c.id === convId)
    if (conv) {
      setCurrentConversationId(convId)
      setMessages(conv.messages || [])
      localStorage.setItem('rag_current_conversation', convId)
    }
  }

  const deleteConversation = (convId) => {
    if (conversations.length === 1) {
      alert('Non puoi eliminare l\'ultima conversazione')
      return
    }

    const updated = conversations.filter(c => c.id !== convId)
    setConversations(updated)
    saveConversationsToStorage(updated)

    if (currentConversationId === convId) {
      loadConversation(updated[0].id)
    }
  }

  const updateConversationTitle = (convId, firstMessage) => {
    const updated = conversations.map(c => {
      if (c.id === convId && c.title === 'Nuova Conversazione') {
        return {
          ...c,
          title: firstMessage.substring(0, 50) + (firstMessage.length > 50 ? '...' : '')
        }
      }
      return c
    })
    setConversations(updated)
    saveConversationsToStorage(updated)
  }

  const updateConversationMessages = (convId, newMessages) => {
    const updated = conversations.map(c =>
      c.id === convId ? { ...c, messages: newMessages } : c
    )
    setConversations(updated)
    saveConversationsToStorage(updated)
  }

  // ============================================================================
  // BACKEND FUNCTIONS
  // ============================================================================

  const checkBackendHealth = async () => {
    try {
      const response = await axios.get(`${API_URL}/health`)
      setBackendHealth(response.data)
      setStatus('ready')
    } catch (error) {
      setBackendHealth(null)
      setStatus('error')
    }
  }

  const fetchDocuments = async () => {
    setLoadingDocuments(true)
    try {
      const response = await axios.get(`${API_URL}/api/documents`)
      const docs = response.data.documents || []
      setDocuments(docs)
    } catch (error) {
      console.error('❌ Errore fetch documenti:', error)
    } finally {
      setLoadingDocuments(false)
    }
  }

  const checkDuplicateDocument = (filename) => {
    return documents.some(doc => doc.filename === filename)
  }

  const pollDocumentsUntilReady = async (initialCount, maxAttempts = 15) => {
    setUploadPhase('⏳ Attesa completamento elaborazione...')

    for (let i = 0; i < maxAttempts; i++) {
      await new Promise(resolve => setTimeout(resolve, 2000))

      try {
        const response = await axios.get(`${API_URL}/api/documents`)
        const currentDocs = response.data.documents || []

        if (currentDocs.length > initialCount) {
          return currentDocs
        }

        setUploadPhase(`⏳ Elaborazione in corso... (${i * 2}s)`)
      } catch (error) {
        console.error('Poll error:', error)
      }
    }

    return null
  }

  const handleFileUpload = async (e) => {
    const file = e.target.files[0]
    if (!file) return

    if (checkDuplicateDocument(file.name)) {
      const confirm = window.confirm(
        `⚠️ Il documento "${file.name}" è già presente.\n\nVuoi caricarlo comunque?`
      )
      if (!confirm) {
        e.target.value = ''
        return
      }
    }

    const initialDocCount = documents.length

    setUploading(true)
    setUploadProgress(0)
    setUploadPhase('📤 Caricamento file...')

    const formData = new FormData()
    formData.append('file', file)

    try {
      const config = {
        onUploadProgress: (progressEvent) => {
          const percent = Math.round(
            (progressEvent.loaded * 100) / progressEvent.total
          )
          setUploadProgress(percent)
        }
      }

      setUploadPhase('📤 Invio al server...')
      const response = await axios.post(`${API_URL}/api/documents/upload`, formData, config)

      setUploadProgress(100)
      setUploadPhase('🔄 Elaborazione documento (OCR → Chunking → Embedding)...')

      const updatedDocs = await pollDocumentsUntilReady(initialDocCount)

      if (updatedDocs) {
        setDocuments(updatedDocs)
        setUploadPhase('✅ Completato!')
        await new Promise(resolve => setTimeout(resolve, 1000))
        alert(`✅ File caricato con successo: ${response.data.filename}\n\nIl documento è ora disponibile per le ricerche.`)
      } else {
        setUploadPhase('⚠️ Elaborazione in corso (continua in background)')
        await fetchDocuments()
        alert(`⚠️ File caricato: ${response.data.filename}\n\nL'elaborazione sta richiedendo più tempo del previsto.\nControlla la lista documenti tra qualche secondo.`)
      }

    } catch (error) {
      console.error('❌ Upload error:', error)
      alert(`❌ Errore upload: ${error.response?.data?.detail || error.message}`)
    } finally {
      setUploading(false)
      setUploadProgress(0)
      setUploadPhase('')
      e.target.value = ''
    }
  }

  const handleQuery = async (e) => {
    e.preventDefault()
    if (!query.trim() || querying) return

    const userMessage = {
      role: 'user',
      content: query,
      timestamp: new Date().toISOString()
    }

    const updatedMessages = [...messages, userMessage]
    setMessages(updatedMessages)
    updateConversationMessages(currentConversationId, updatedMessages)

    if (updatedMessages.length === 1) {
      updateConversationTitle(currentConversationId, query)
    }

    setQuery('')
    setQuerying(true)
    setIsModelLoading(false)

    // Timer: dopo 5 secondi mostra messaggio caricamento modello
    modelLoadingTimerRef.current = setTimeout(() => {
      setIsModelLoading(true)
    }, 5000)

    try {
      const response = await axios.post(`${API_URL}/api/query`, {
        query: userMessage.content,
        top_k: 5
      })

      const assistantMessage = {
        role: 'assistant',
        content: response.data.answer,
        sources: response.data.sources || [],
        timestamp: new Date().toISOString()
      }

      const finalMessages = [...updatedMessages, assistantMessage]
      setMessages(finalMessages)
      updateConversationMessages(currentConversationId, finalMessages)

    } catch (error) {
      console.error('Query error:', error)

      const errorMessage = {
        role: 'assistant',
        content: `❌ Errore: ${error.response?.data?.detail || error.message}`,
        error: true,
        timestamp: new Date().toISOString()
      }

      const finalMessages = [...updatedMessages, errorMessage]
      setMessages(finalMessages)
      updateConversationMessages(currentConversationId, finalMessages)
    } finally {
      // Pulisci timer e reset stati
      if (modelLoadingTimerRef.current) {
        clearTimeout(modelLoadingTimerRef.current)
        modelLoadingTimerRef.current = null
      }
      setQuerying(false)
      setIsModelLoading(false)
    }
  }

  const handleDeleteDocument = async (documentId) => {
    if (!window.confirm('Sei sicuro di voler eliminare questo documento?')) {
      return
    }

    try {
      await axios.delete(`${API_URL}/api/documents/${documentId}`)
      alert('✅ Documento eliminato')
      fetchDocuments()
    } catch (error) {
      console.error('Delete error:', error)
      alert(`❌ Errore eliminazione: ${error.response?.data?.detail || error.message}`)
    }
  }

  // ============================================================================
  // RENDER: LOGIN SCREEN
  // ============================================================================

  if (!isAuthenticated) {
    return (
      <div className="flex items-center justify-center min-h-screen bg-gradient-to-br from-slate-900 to-slate-800">
        <div className="w-full max-w-md p-8 bg-slate-800 rounded-lg shadow-2xl border border-slate-700">
          <div className="text-center mb-8">
            {BRANDING.clientLogo ? (
              <img src={BRANDING.clientLogo} alt="Logo" className="h-16 mx-auto mb-4" />
            ) : (
              <h1 className="text-3xl font-bold text-white mb-2">{BRANDING.clientName}</h1>
            )}
            <p className="text-slate-400">Accedi per continuare</p>
          </div>

          <form onSubmit={handleLogin} className="space-y-4">
            <div>
              <label className="block text-sm font-medium text-slate-300 mb-2">
                Username
              </label>
              <input
                type="text"
                value={loginForm.username}
                onChange={(e) => setLoginForm({ ...loginForm, username: e.target.value })}
                className="w-full px-4 py-2 bg-slate-700 text-white rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
                placeholder="admin"
                required
              />
            </div>

            <div>
              <label className="block text-sm font-medium text-slate-300 mb-2">
                Password
              </label>
              <input
                type="password"
                value={loginForm.password}
                onChange={(e) => setLoginForm({ ...loginForm, password: e.target.value })}
                className="w-full px-4 py-2 bg-slate-700 text-white rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
                placeholder="••••••••"
                required
              />
            </div>

            {loginError && (
              <div className="p-3 bg-red-900/30 border border-red-500 rounded-lg text-red-200 text-sm">
                {loginError}
              </div>
            )}

            <button
              type="submit"
              disabled={loggingIn}
              className="w-full py-3 bg-blue-600 hover:bg-blue-700 disabled:bg-slate-600 text-white font-semibold rounded-lg transition"
            >
              {loggingIn ? 'Accesso in corso...' : 'Accedi'}
            </button>
          </form>

          <div className="mt-6 pt-6 border-t border-slate-700 text-center text-xs text-slate-400">
            <p>Credenziali di default:</p>
            <p className="mt-1">Username: <span className="text-white font-mono">admin</span></p>
            <p>Password: <span className="text-white font-mono">admin123</span></p>
          </div>
        </div>
      </div>
    )
  }

  // ============================================================================
  // RENDER: ADMIN PANEL (MODAL)
  // ============================================================================

  const canUploadDelete = user && (user.role === 'admin' || user.role === 'super_user')
  const isAdmin = user && user.role === 'admin'

  return (
    <div className="flex flex-col h-screen bg-gradient-to-br from-slate-900 to-slate-800">

      {/* ADMIN PANEL MODAL */}
      {showAdminPanel && isAdmin && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-slate-800 rounded-lg shadow-2xl border border-slate-700 w-full max-w-4xl max-h-[90vh] overflow-hidden flex flex-col">
            {/* Header */}
            <div className="p-6 border-b border-slate-700 flex items-center justify-between">
              <h2 className="text-2xl font-bold text-white">👥 Gestione Utenti</h2>
              <button
                onClick={toggleAdminPanel}
                className="text-slate-400 hover:text-white text-2xl"
              >
                ✕
              </button>
            </div>

            {/* Content */}
            <div className="flex-1 overflow-y-auto p-6 space-y-6">
              {/* Create User Form */}
              <div className="bg-slate-700 rounded-lg p-4">
                <h3 className="text-lg font-semibold text-white mb-4">➕ Crea Nuovo Utente</h3>
                <form onSubmit={handleCreateUser} className="grid grid-cols-2 gap-4">
                  <input
                    type="text"
                    placeholder="Username"
                    value={newUserForm.username}
                    onChange={(e) => setNewUserForm({ ...newUserForm, username: e.target.value })}
                    className="px-3 py-2 bg-slate-600 text-white rounded focus:outline-none focus:ring-2 focus:ring-blue-500"
                    required
                  />
                  <input
                    type="email"
                    placeholder="Email"
                    value={newUserForm.email}
                    onChange={(e) => setNewUserForm({ ...newUserForm, email: e.target.value })}
                    className="px-3 py-2 bg-slate-600 text-white rounded focus:outline-none focus:ring-2 focus:ring-blue-500"
                    required
                  />
                  <input
                    type="password"
                    placeholder="Password"
                    value={newUserForm.password}
                    onChange={(e) => setNewUserForm({ ...newUserForm, password: e.target.value })}
                    className="px-3 py-2 bg-slate-600 text-white rounded focus:outline-none focus:ring-2 focus:ring-blue-500"
                    required
                  />
                  <select
                    value={newUserForm.role}
                    onChange={(e) => setNewUserForm({ ...newUserForm, role: e.target.value })}
                    className="px-3 py-2 bg-slate-600 text-white rounded focus:outline-none focus:ring-2 focus:ring-blue-500"
                  >
                    <option value="user">User (solo consultazione)</option>
                    <option value="super_user">Super User (upload/delete)</option>
                    <option value="admin">Admin (gestione utenti)</option>
                  </select>
                  <button
                    type="submit"
                    disabled={creatingUser}
                    className="col-span-2 py-2 bg-green-600 hover:bg-green-700 disabled:bg-slate-600 text-white font-semibold rounded transition"
                  >
                    {creatingUser ? 'Creazione...' : 'Crea Utente'}
                  </button>
                </form>
              </div>

              {/* Users List */}
              <div>
                <div className="flex items-center justify-between mb-4">
                  <h3 className="text-lg font-semibold text-white">📋 Utenti Registrati ({allUsers.length})</h3>
                  <button
                    onClick={fetchAllUsers}
                    className="text-sm text-blue-400 hover:text-blue-300"
                  >
                    🔄 Aggiorna
                  </button>
                </div>

                {loadingUsers ? (
                  <p className="text-center text-slate-400 py-8">Caricamento...</p>
                ) : allUsers.length === 0 ? (
                  <p className="text-center text-slate-400 py-8">Nessun utente trovato</p>
                ) : (
                  <div className="space-y-2">
                    {allUsers.map(u => (
                      <div key={u.id} className="bg-slate-700 rounded-lg p-4 flex items-center justify-between">
                        <div className="flex-1">
                          <div className="flex items-center gap-3">
                            <p className="text-white font-semibold">{u.username}</p>
                            <span className={`px-2 py-1 rounded text-xs font-bold ${
                              u.role === 'admin' ? 'bg-red-600 text-white' :
                              u.role === 'super_user' ? 'bg-purple-600 text-white' :
                              'bg-blue-600 text-white'
                            }`}>
                              {u.role.toUpperCase()}
                            </span>
                          </div>
                          <p className="text-sm text-slate-400 mt-1">{u.email}</p>
                        </div>

                        <div className="flex items-center gap-2">
                          {u.id !== user.id && (
                            <>
                              <select
                                value={u.role}
                                onChange={(e) => handleChangeUserRole(u.id, e.target.value, u.username)}
                                className="px-2 py-1 bg-slate-600 text-white text-sm rounded"
                              >
                                <option value="user">User</option>
                                <option value="super_user">Super User</option>
                                <option value="admin">Admin</option>
                              </select>
                              <button
                                onClick={() => handleDeleteUser(u.id, u.username)}
                                className="px-3 py-1 bg-red-600 hover:bg-red-700 text-white text-sm rounded transition"
                              >
                                🗑️ Elimina
                              </button>
                            </>
                          )}
                          {u.id === user.id && (
                            <span className="text-sm text-slate-400 italic">(Tu)</span>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>
      )}

      {/* CHANGE PASSWORD MODAL */}
      {showChangePasswordModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-slate-800 rounded-lg shadow-2xl border border-slate-700 w-full max-w-md">
            {/* Header */}
            <div className="p-6 border-b border-slate-700 flex items-center justify-between">
              <h2 className="text-2xl font-bold text-white">🔑 Cambia Password</h2>
              <button
                onClick={toggleChangePasswordModal}
                className="text-slate-400 hover:text-white text-2xl"
              >
                ✕
              </button>
            </div>

            {/* Content */}
            <div className="p-6">
              <form onSubmit={handleChangePassword} className="space-y-4">
                <div>
                  <label className="block text-sm font-medium text-slate-300 mb-2">
                    Password Attuale
                  </label>
                  <input
                    type="password"
                    value={passwordForm.oldPassword}
                    onChange={(e) => setPasswordForm({ ...passwordForm, oldPassword: e.target.value })}
                    className="w-full px-3 py-2 bg-slate-700 text-white rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
                    placeholder="••••••••"
                    required
                  />
                </div>

                <div>
                  <label className="block text-sm font-medium text-slate-300 mb-2">
                    Nuova Password
                  </label>
                  <input
                    type="password"
                    value={passwordForm.newPassword}
                    onChange={(e) => setPasswordForm({ ...passwordForm, newPassword: e.target.value })}
                    className="w-full px-3 py-2 bg-slate-700 text-white rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
                    placeholder="••••••••"
                    required
                    minLength={6}
                  />
                  <p className="text-xs text-slate-400 mt-1">Minimo 6 caratteri</p>
                </div>

                <div>
                  <label className="block text-sm font-medium text-slate-300 mb-2">
                    Conferma Nuova Password
                  </label>
                  <input
                    type="password"
                    value={passwordForm.confirmPassword}
                    onChange={(e) => setPasswordForm({ ...passwordForm, confirmPassword: e.target.value })}
                    className="w-full px-3 py-2 bg-slate-700 text-white rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
                    placeholder="••••••••"
                    required
                    minLength={6}
                  />
                </div>

                {passwordError && (
                  <div className="p-3 bg-red-900/30 border border-red-500 rounded-lg text-red-200 text-sm">
                    {passwordError}
                  </div>
                )}

                <div className="flex gap-3 pt-2">
                  <button
                    type="button"
                    onClick={toggleChangePasswordModal}
                    className="flex-1 py-2 bg-slate-700 hover:bg-slate-600 text-white font-semibold rounded-lg transition"
                  >
                    Annulla
                  </button>
                  <button
                    type="submit"
                    disabled={changingPassword}
                    className="flex-1 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-slate-600 text-white font-semibold rounded-lg transition"
                  >
                    {changingPassword ? 'Aggiornamento...' : 'Cambia Password'}
                  </button>
                </div>
              </form>
            </div>
          </div>
        </div>
      )}

      {/* HEADER */}
      <header className="bg-slate-800 border-b border-slate-700 px-6 py-4 flex items-center justify-between">
        <div className="flex items-center gap-4">
          {BRANDING.clientLogo ? (
            <img src={BRANDING.clientLogo} alt="Logo" className="h-10" />
          ) : (
            <h1 className="text-2xl font-bold text-white">{BRANDING.clientName}</h1>
          )}
        </div>

        {/* User info and status */}
        <div className="flex items-center gap-6">
          {/* Backend status */}
          <div className="flex items-center gap-2">
            <div className={`w-3 h-3 rounded-full ${
              status === 'ready' ? 'bg-green-500' :
              status === 'error' ? 'bg-red-500' :
              'bg-yellow-500'
            }`} />
            <span className="text-sm text-slate-300">
              {status === 'ready' ? 'Online' :
               status === 'error' ? 'Offline' :
               'Checking...'}
            </span>
          </div>

          {/* User info */}
          <div className="flex items-center gap-3 border-l border-slate-700 pl-6">
            <div className="text-right">
              <p className="text-sm font-semibold text-white">{user.username}</p>
              <p className={`text-xs ${
                user.role === 'admin' ? 'text-red-400' :
                user.role === 'super_user' ? 'text-purple-400' :
                'text-blue-400'
              }`}>
                {user.role === 'admin' ? '👑 Admin' :
                 user.role === 'super_user' ? '⭐ Super User' :
                 '👤 User'}
              </p>
            </div>

            {isAdmin && (
              <button
                onClick={toggleAdminPanel}
                className="px-3 py-1 bg-slate-700 hover:bg-slate-600 text-white text-sm rounded transition"
                title="Gestione utenti"
              >
                👥
              </button>
            )}

            <button
              onClick={toggleChangePasswordModal}
              className="px-3 py-1 bg-slate-700 hover:bg-slate-600 text-white text-sm rounded transition"
              title="Cambia password"
            >
              🔑
            </button>

            <button
              onClick={handleLogout}
              className="px-3 py-1 bg-red-600 hover:bg-red-700 text-white text-sm rounded transition"
            >
              Logout
            </button>
          </div>
        </div>
      </header>

      {/* MAIN CONTENT */}
      <div className="flex flex-1 overflow-hidden">

        {/* SIDEBAR CONVERSAZIONI */}
        {showConversationsSidebar && (
          <aside className="w-64 bg-slate-800 border-r border-slate-700 flex flex-col">
            <div className="p-4 border-b border-slate-700">
              <button
                onClick={createNewConversation}
                className="w-full py-2 px-4 bg-blue-600 hover:bg-blue-700 text-white rounded-lg font-semibold transition"
              >
                + Nuova Chat
              </button>
            </div>

            <div className="flex-1 overflow-y-auto p-2 space-y-1">
              {conversations.map(conv => (
                <div
                  key={conv.id}
                  className={`group flex items-center justify-between p-3 rounded cursor-pointer transition ${
                    currentConversationId === conv.id
                      ? 'bg-slate-700 text-white'
                      : 'text-slate-300 hover:bg-slate-700/50'
                  }`}
                  onClick={() => loadConversation(conv.id)}
                >
                  <span className="truncate flex-1 text-sm">{conv.title}</span>
                  {conversations.length > 1 && (
                    <button
                      onClick={(e) => {
                        e.stopPropagation()
                        deleteConversation(conv.id)
                      }}
                      className="opacity-0 group-hover:opacity-100 text-red-400 hover:text-red-300 text-xs ml-2"
                    >
                      🗑️
                    </button>
                  )}
                </div>
              ))}
            </div>

            <div className="p-2 border-t border-slate-700">
              <button
                onClick={() => setShowConversationsSidebar(false)}
                className="w-full py-1 text-xs text-slate-400 hover:text-white"
              >
                ◀ Chiudi
              </button>
            </div>
          </aside>
        )}

        {/* CHAT AREA */}
        <main className="flex-1 flex flex-col">

          {!showConversationsSidebar && (
            <button
              onClick={() => setShowConversationsSidebar(true)}
              className="absolute top-20 left-4 p-2 bg-slate-700 text-white rounded-lg shadow-lg hover:bg-slate-600 z-10"
            >
              ▶
            </button>
          )}

          {/* Messages area */}
          <div className="flex-1 overflow-y-auto p-6 space-y-4">
            {messages.length === 0 ? (
              <div className="flex items-center justify-center h-full">
                <div className="text-center text-slate-400">
                  <h2 className="text-2xl font-bold mb-2">👋 Ciao, {user.username}!</h2>
                  <p>Inizia una conversazione{canUploadDelete ? ' oppure carica dei documenti per iniziare' : ''}.</p>
                </div>
              </div>
            ) : (
              messages.map((msg, idx) => (
                <div
                  key={idx}
                  className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}
                >
                  <div
                    className={`max-w-3xl rounded-lg p-4 ${
                      msg.role === 'user'
                        ? 'bg-blue-600 text-white'
                        : msg.error
                        ? 'bg-red-900/30 border border-red-500 text-red-200'
                        : 'bg-slate-700 text-slate-100'
                    }`}
                  >
                    <p className="whitespace-pre-wrap leading-relaxed">{msg.content}</p>

                    {msg.sources && msg.sources.length > 0 && (
                      <div className="mt-4 pt-4 border-t border-slate-600 space-y-2">
                        <p className="text-sm font-semibold text-slate-300">
                          📚 Fonti ({msg.sources.length}):
                        </p>
                        {msg.sources.map((source, sidx) => (
                          <div key={sidx} className="bg-slate-600 rounded p-2 text-sm">
                            <div className="flex justify-between items-center gap-2">
                              <a
                                href={`${API_URL}/api/documents/${source.document_id}/download`}
                                download
                                className="text-blue-300 hover:text-blue-200 underline truncate flex-1"
                                title={source.filename || source.document_id}
                              >
                                {source.filename || source.document_id}
                              </a>
                              <span className="bg-green-600 text-white px-2 py-1 rounded text-xs font-bold">
                                {source.similarity_score ? (source.similarity_score * 100).toFixed(1) : 'N/A'}%
                              </span>
                            </div>
                          </div>
                        ))}
                      </div>
                    )}

                    <p className="text-xs text-slate-400 mt-2">
                      {new Date(msg.timestamp).toLocaleTimeString('it-IT')}
                    </p>
                  </div>
                </div>
              ))
            )}

            {querying && (
              <div className="flex justify-start">
                <div className="bg-slate-700 rounded-lg p-4 text-slate-300">
                  <div className="flex flex-col gap-2">
                    <div className="flex items-center gap-2">
                      <div className="animate-spin">⏳</div>
                      <span>
                        {isModelLoading
                          ? '🧠 Il modello LLM si sta caricando in memoria...'
                          : 'Ricerca in corso...'}
                      </span>
                    </div>
                    {isModelLoading && (
                      <p className="text-xs text-slate-400 ml-6">
                        Può richiedere 10-20 secondi al primo avvio o dopo inattività
                      </p>
                    )}
                  </div>
                </div>
              </div>
            )}

            <div ref={messagesEndRef} />
          </div>

          {/* Input area */}
          <div className="border-t border-slate-700 p-4 bg-slate-800">
            <form onSubmit={handleQuery} className="flex gap-2">
              <input
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Scrivi la tua domanda..."
                disabled={querying || status !== 'ready'}
                className="flex-1 bg-slate-700 text-white placeholder-slate-400 rounded-lg px-4 py-3 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
              />
              <button
                type="submit"
                disabled={querying || !query.trim() || status !== 'ready'}
                className="px-6 py-3 bg-blue-600 hover:bg-blue-700 disabled:bg-slate-600 disabled:cursor-not-allowed text-white rounded-lg font-semibold transition"
              >
                {querying ? '⏳' : '🚀'}
              </button>
            </form>
          </div>
        </main>

        {/* SIDEBAR DOCUMENTI */}
        {showDocumentsSidebar && (
          <aside className="w-80 bg-slate-800 border-l border-slate-700 flex flex-col">
            <div className="p-4 border-b border-slate-700">
              <h2 className="text-lg font-bold text-white mb-3">📁 Documenti</h2>

              {canUploadDelete && (
                <>
                  <input
                    ref={fileInputRef}
                    type="file"
                    onChange={handleFileUpload}
                    disabled={uploading}
                    className="hidden"
                    accept=".pdf,.docx,.txt,.doc,.pptx,.xlsx"
                  />

                  <button
                    onClick={() => fileInputRef.current?.click()}
                    disabled={uploading}
                    className="w-full py-2 px-4 bg-green-600 hover:bg-green-700 disabled:bg-slate-600 text-white rounded-lg font-semibold transition"
                  >
                    {uploading ? `⏳ ${uploadProgress}%` : '+ Carica File'}
                  </button>

                  {uploading && (
                    <div className="mt-3 space-y-2">
                      <div className="bg-slate-700 rounded-full h-2">
                        <div
                          className="bg-green-500 h-2 rounded-full transition-all"
                          style={{ width: `${uploadProgress}%` }}
                        />
                      </div>

                      {uploadPhase && (
                        <p className="text-sm text-slate-300 text-center animate-pulse">
                          {uploadPhase}
                        </p>
                      )}
                    </div>
                  )}
                </>
              )}
            </div>

            <div className="flex-1 overflow-y-auto p-3">
              {loadingDocuments ? (
                <p className="text-center text-slate-400 py-4">Caricamento...</p>
              ) : documents.length === 0 ? (
                <p className="text-center text-slate-400 py-4 text-sm">
                  Nessun documento caricato
                </p>
              ) : (
                <div className="space-y-2">
                  {documents.map((doc, idx) => (
                    <div
                      key={idx}
                      className="bg-slate-700 rounded-lg p-3 group hover:bg-slate-600 transition"
                    >
                      <div className="flex items-start justify-between gap-2">
                        <div className="flex-1 min-w-0">
                          <p className="text-white text-sm font-semibold truncate" title={doc.filename}>
                            {doc.filename}
                          </p>
                          <p className="text-slate-400 text-xs mt-1">
                            {doc.num_chunks || 0} chunks
                          </p>
                        </div>

                        {canUploadDelete && (
                          <button
                            onClick={() => handleDeleteDocument(doc.document_id)}
                            className="opacity-0 group-hover:opacity-100 text-red-400 hover:text-red-300 text-xs transition"
                            title="Elimina documento"
                          >
                            🗑️
                          </button>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>

            <div className="p-2 border-t border-slate-700">
              <button
                onClick={fetchDocuments}
                className="w-full py-2 text-sm text-slate-400 hover:text-white transition"
              >
                🔄 Aggiorna Lista
              </button>
              <button
                onClick={() => setShowDocumentsSidebar(false)}
                className="w-full py-1 text-xs text-slate-400 hover:text-white mt-1"
              >
                Chiudi ▶
              </button>
            </div>
          </aside>
        )}

        {!showDocumentsSidebar && (
          <button
            onClick={() => setShowDocumentsSidebar(true)}
            className="absolute top-20 right-4 p-2 bg-slate-700 text-white rounded-lg shadow-lg hover:bg-slate-600 z-10"
          >
            ◀
          </button>
        )}
      </div>

      {/* FOOTER */}
      <footer className="bg-slate-900 border-t border-slate-700 px-6 py-3">
        <div className="flex items-center justify-between text-xs text-slate-400">
          <div>
            <span>Powered by </span>
            <span className="font-semibold text-blue-400">
              {BRANDING.poweredBy}{BRANDING.poweredBySubtitle ? ` ${BRANDING.poweredBySubtitle}` : ''}
            </span>
            <span className="mx-2">•</span>
            <span>{BRANDING.version}</span>
          </div>
          <div>
            <a
              href="#"
              className="hover:text-white transition"
              onClick={(e) => {
                e.preventDefault()
                alert('⚠️ Disclaimer:\n\nQuesto sistema utilizza intelligenza artificiale per analizzare documenti e fornire risposte. Le informazioni fornite devono essere verificate e non sostituiscono la consulenza professionale.\n\n© I3K Technologies - Tutti i diritti riservati.')
              }}
            >
              Disclaimer
            </a>
            <span className="mx-2">•</span>
            <a href="#" className="hover:text-white transition">Privacy</a>
          </div>
        </div>
      </footer>
    </div>
  )
}

export default App
