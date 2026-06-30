import { useState, useRef, useEffect } from 'react'
import axios from 'axios'
import './index.css'

const BRANDING = {
  clientLogo: null,
  clientName: 'i3k RAG Engine',
  primaryColor: '#3b82f6',
  poweredBy: 'I3K Technologies',
  poweredBySubtitle: 'Ltd.',
  version: 'Community'
}

const API_URL = import.meta.env.VITE_API_URL || ''

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
  const [adminTab, setAdminTab] = useState('users')
  const [allUsers, setAllUsers] = useState([])
  const [loadingUsers, setLoadingUsers] = useState(false)
  const [newUserForm, setNewUserForm] = useState({ username: '', email: '', password: '', role: 'user' })
  const [creatingUser, setCreatingUser] = useState(false)

  // Backup state (simplified — local archives only)
  const [localBackups, setLocalBackups] = useState([])
  const [backupRunning, setBackupRunning] = useState(false)

  // Change password state
  const [showChangePasswordModal, setShowChangePasswordModal] = useState(false)
  const [passwordForm, setPasswordForm] = useState({ oldPassword: '', newPassword: '', confirmPassword: '' })
  const [changingPassword, setChangingPassword] = useState(false)
  const [passwordError, setPasswordError] = useState('')

  // Backend status
  const [status, setStatus] = useState('checking')

  // Conversations (localStorage)
  const [conversations, setConversations] = useState([])
  const [currentConversationId, setCurrentConversationId] = useState(null)
  const [messages, setMessages] = useState([])

  // Input query
  const [query, setQuery] = useState('')
  const [querying, setQuerying] = useState(false)
  const [isModelLoading, setIsModelLoading] = useState(false)

  // Documents
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

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }

  useEffect(scrollToBottom, [messages])

  useEffect(() => {
    const savedToken = localStorage.getItem('rag_auth_token')
    const savedUser = localStorage.getItem('rag_auth_user')
    if (savedToken && savedUser) {
      setToken(savedToken)
      setUser(JSON.parse(savedUser))
      setIsAuthenticated(true)
    }
  }, [])

  useEffect(() => {
    if (isAuthenticated) {
      loadConversationsFromStorage()
      checkBackendHealth()
      fetchDocuments()
      const interval = setInterval(checkBackendHealth, 30000)
      return () => clearInterval(interval)
    }
  }, [isAuthenticated])

  useEffect(() => {
    const reqInt = axios.interceptors.request.use(
      (config) => {
        if (token) config.headers.Authorization = `Bearer ${token}`
        return config
      },
      (error) => Promise.reject(error)
    )
    const resInt = axios.interceptors.response.use(
      (response) => response,
      (error) => {
        if (error.response?.status === 401) handleLogout()
        return Promise.reject(error)
      }
    )
    return () => {
      axios.interceptors.request.eject(reqInt)
      axios.interceptors.response.eject(resInt)
    }
  }, [token])

  // ============================================================================
  // AUTHENTICATION
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
      localStorage.setItem('rag_auth_token', access_token)
      localStorage.setItem('rag_auth_user', JSON.stringify(userData))
    } catch (error) {
      setLoginError(error.response?.data?.error || 'Login error')
    } finally {
      setLoggingIn(false)
    }
  }

  const handleLogout = () => {
    setToken(null)
    setUser(null)
    setIsAuthenticated(false)
    setConversations([])
    setMessages([])
    setCurrentConversationId(null)
    localStorage.removeItem('rag_auth_token')
    localStorage.removeItem('rag_auth_user')
    setLoginForm({ username: '', password: '' })
    setShowAdminPanel(false)
  }

  // ============================================================================
  // ADMIN — USERS
  // ============================================================================

  const fetchAllUsers = async () => {
    if (!user || user.role !== 'admin') return
    setLoadingUsers(true)
    try {
      const response = await axios.get(`${API_URL}/api/auth/users`)
      setAllUsers(response.data.users || [])
    } catch (error) {
      alert('Error loading users')
    } finally {
      setLoadingUsers(false)
    }
  }

  const handleCreateUser = async (e) => {
    e.preventDefault()
    setCreatingUser(true)
    try {
      await axios.post(`${API_URL}/api/auth/users`, newUserForm)
      alert(`User "${newUserForm.username}" created successfully!`)
      setNewUserForm({ username: '', email: '', password: '', role: 'user' })
      fetchAllUsers()
    } catch (error) {
      alert(`Error: ${error.response?.data?.error || error.message}`)
    } finally {
      setCreatingUser(false)
    }
  }

  const handleDeleteUser = async (userId, username) => {
    if (!window.confirm(`Delete user "${username}"?`)) return
    try {
      await axios.delete(`${API_URL}/api/auth/users/${userId}`)
      alert('User deleted')
      fetchAllUsers()
    } catch (error) {
      alert(`Error: ${error.response?.data?.error || error.message}`)
    }
  }

  const handleChangeUserRole = async (userId, newRole, username) => {
    try {
      await axios.put(`${API_URL}/api/auth/users/${userId}`, { role: newRole })
      alert(`Role of "${username}" updated to "${newRole}"`)
      fetchAllUsers()
    } catch (error) {
      alert(`Error: ${error.response?.data?.error || error.message}`)
    }
  }

  const toggleAdminPanel = () => {
    const next = !showAdminPanel
    setShowAdminPanel(next)
    if (next) fetchAllUsers()
  }

  // ============================================================================
  // ADMIN — BACKUP
  // ============================================================================

  const fetchBackupList = async () => {
    try {
      const res = await axios.get(`${API_URL}/api/admin/backup/list`)
      setLocalBackups(res.data.backups || [])
    } catch (error) {
      console.error('Error fetching backups:', error)
    }
  }

  const handleRunBackup = async () => {
    setBackupRunning(true)
    try {
      const res = await axios.post(`${API_URL}/api/admin/backup`)
      alert(`Backup created: ${res.data.archive}`)
      fetchBackupList()
    } catch (error) {
      alert(`Backup error: ${error.response?.data?.error || error.message}`)
    } finally {
      setBackupRunning(false)
    }
  }

  // ============================================================================
  // PASSWORD CHANGE
  // ============================================================================

  const handleChangePassword = async (e) => {
    e.preventDefault()
    setPasswordError('')
    if (passwordForm.newPassword !== passwordForm.confirmPassword) {
      setPasswordError('New passwords do not match')
      return
    }
    if (passwordForm.newPassword.length < 6) {
      setPasswordError('New password must be at least 6 characters')
      return
    }
    setChangingPassword(true)
    try {
      await axios.post(`${API_URL}/api/auth/change-password`, {
        old_password: passwordForm.oldPassword,
        new_password: passwordForm.newPassword
      })
      alert('Password changed successfully!')
      setShowChangePasswordModal(false)
      setPasswordForm({ oldPassword: '', newPassword: '', confirmPassword: '' })
    } catch (error) {
      setPasswordError(error.response?.data?.error || 'Error changing password')
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
  // CONVERSATIONS (localStorage)
  // ============================================================================

  const loadConversationsFromStorage = () => {
    if (!user) return
    try {
      const stored = localStorage.getItem(`rag_conversations_${user.id}`)
      if (stored) {
        const parsed = JSON.parse(stored)
        setConversations(parsed)
        const lastId = localStorage.getItem(`rag_current_conversation_${user.id}`)
        if (lastId && parsed.find(c => c.id === lastId)) {
          loadConversation(lastId, parsed)
        } else if (parsed.length > 0) {
          loadConversation(parsed[0].id, parsed)
        }
      } else {
        createNewConversation([])
      }
    } catch {
      createNewConversation([])
    }
  }

  const saveConversationsToStorage = (convs) => {
    if (!user) return
    localStorage.setItem(`rag_conversations_${user.id}`, JSON.stringify(convs))
  }

  const createNewConversation = (existingConvs) => {
    const base = existingConvs !== undefined ? existingConvs : conversations
    const newConv = {
      id: Date.now().toString(),
      title: 'New Conversation',
      messages: [],
      createdAt: new Date().toISOString()
    }
    const updated = [newConv, ...base]
    setConversations(updated)
    saveConversationsToStorage(updated)
    setCurrentConversationId(newConv.id)
    setMessages([])
  }

  const loadConversation = (convId, convList) => {
    const list = convList || conversations
    const conv = list.find(c => c.id === convId)
    if (conv && user) {
      setCurrentConversationId(convId)
      setMessages(conv.messages || [])
      localStorage.setItem(`rag_current_conversation_${user.id}`, convId)
    }
  }

  const deleteConversation = (convId) => {
    if (conversations.length === 1) {
      alert('Cannot delete the last conversation')
      return
    }
    const updated = conversations.filter(c => c.id !== convId)
    setConversations(updated)
    saveConversationsToStorage(updated)
    if (currentConversationId === convId) {
      loadConversation(updated[0].id, updated)
    }
  }

  const updateConversationTitle = (convId, firstMessage) => {
    setConversations(prev => {
      const updated = prev.map(c => {
        if (c.id === convId && c.title === 'New Conversation') {
          return { ...c, title: firstMessage.substring(0, 50) + (firstMessage.length > 50 ? '...' : '') }
        }
        return c
      })
      saveConversationsToStorage(updated)
      return updated
    })
  }

  const updateConversationMessages = (convId, newMessages) => {
    setConversations(prev => {
      const updated = prev.map(c => c.id === convId ? { ...c, messages: newMessages } : c)
      saveConversationsToStorage(updated)
      return updated
    })
  }

  // ============================================================================
  // BACKEND / DOCUMENTS
  // ============================================================================

  const checkBackendHealth = async () => {
    try {
      await axios.get(`${API_URL}/health`)
      setStatus('ready')
    } catch {
      setStatus('error')
    }
  }

  const fetchDocuments = async () => {
    setLoadingDocuments(true)
    try {
      const response = await axios.get(`${API_URL}/api/documents`)
      setDocuments(response.data.documents || [])
    } catch (error) {
      console.error('Error fetching documents:', error)
    } finally {
      setLoadingDocuments(false)
    }
  }

  const pollDocumentsUntilReady = async (initialCount, maxAttempts = 15) => {
    setUploadPhase('Waiting for processing to complete...')
    for (let i = 0; i < maxAttempts; i++) {
      await new Promise(resolve => setTimeout(resolve, 2000))
      try {
        const response = await axios.get(`${API_URL}/api/documents`)
        const currentDocs = response.data.documents || []
        if (currentDocs.length > initialCount) return currentDocs
        setUploadPhase(`Processing... (${i * 2}s)`)
      } catch { /* continue */ }
    }
    return null
  }

  const handleFileUpload = async (e) => {
    const file = e.target.files[0]
    if (!file) return

    if (documents.some(doc => doc.filename === file.name)) {
      if (!window.confirm(`"${file.name}" already exists. Upload anyway?`)) {
        e.target.value = ''
        return
      }
    }

    const initialDocCount = documents.length
    setUploading(true)
    setUploadProgress(0)
    setUploadPhase('Uploading file...')

    const formData = new FormData()
    formData.append('file', file)

    try {
      const response = await axios.post(`${API_URL}/api/documents/upload`, formData, {
        onUploadProgress: (evt) => {
          setUploadProgress(Math.round((evt.loaded * 100) / evt.total))
        }
      })
      setUploadProgress(100)
      setUploadPhase('Processing (OCR → Chunking → Embedding)...')

      const updatedDocs = await pollDocumentsUntilReady(initialDocCount)
      if (updatedDocs) {
        setDocuments(updatedDocs)
        setUploadPhase('Completed!')
        await new Promise(resolve => setTimeout(resolve, 1000))
        alert(`File uploaded: ${response.data.filename}\n\nNow available for search.`)
      } else {
        setUploadPhase('Processing (continues in background)')
        await fetchDocuments()
        alert(`File uploaded: ${response.data.filename}\n\nProcessing is taking longer than expected.`)
      }
    } catch (error) {
      alert(`Upload error: ${error.response?.data?.error || error.message}`)
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

    const userMessage = { role: 'user', content: query, timestamp: new Date().toISOString() }
    const updatedMessages = [...messages, userMessage]
    setMessages(updatedMessages)
    updateConversationMessages(currentConversationId, updatedMessages)
    if (updatedMessages.length === 1) updateConversationTitle(currentConversationId, query)

    setQuery('')
    setQuerying(true)
    setIsModelLoading(false)

    modelLoadingTimerRef.current = setTimeout(() => setIsModelLoading(true), 5000)

    try {
      const response = await axios.post(`${API_URL}/api/query`, {
        query: userMessage.content,
        top_k: 5
      }, { timeout: 630000 })

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
      const isTimeout = error.code === 'ECONNABORTED' || error.message?.includes('timeout')
      const errorContent = isTimeout
        ? 'The model took too long to respond. Please try again.'
        : `Error: ${error.response?.data?.error || error.message}`

      const errorMessage = { role: 'assistant', content: errorContent, error: true, timestamp: new Date().toISOString() }
      const finalMessages = [...updatedMessages, errorMessage]
      setMessages(finalMessages)
      updateConversationMessages(currentConversationId, finalMessages)
    } finally {
      if (modelLoadingTimerRef.current) {
        clearTimeout(modelLoadingTimerRef.current)
        modelLoadingTimerRef.current = null
      }
      setQuerying(false)
      setIsModelLoading(false)
    }
  }

  const handleDeleteDocument = async (documentId) => {
    if (!window.confirm('Delete this document?')) return
    try {
      await axios.delete(`${API_URL}/api/documents/${documentId}`)
      alert('Document deleted')
      fetchDocuments()
    } catch (error) {
      alert(`Delete error: ${error.response?.data?.error || error.message}`)
    }
  }

  // ============================================================================
  // RENDER: LOGIN
  // ============================================================================

  if (!isAuthenticated) {
    return (
      <div className="flex items-center justify-center min-h-screen bg-gradient-to-br from-slate-900 to-slate-800">
        <div className="w-full max-w-md p-8 bg-slate-800 rounded-lg shadow-2xl border border-slate-700">
          <div className="text-center mb-8">
            <h1 className="text-3xl font-bold text-white mb-2">{BRANDING.clientName}</h1>
            <p className="text-slate-400">Sign in to continue</p>
          </div>

          <form onSubmit={handleLogin} className="space-y-4">
            <div>
              <label className="block text-sm font-medium text-slate-300 mb-2">Username</label>
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
              <label className="block text-sm font-medium text-slate-300 mb-2">Password</label>
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
              {loggingIn ? 'Signing in...' : 'Sign In'}
            </button>
          </form>

          <div className="mt-6 pt-6 border-t border-slate-700 text-center text-xs text-slate-400">
            <p>Username: <span className="text-white font-mono">admin</span></p>
            <p className="mt-1">Password: check startup logs or set <span className="text-white font-mono">AUTH__ADMIN_DEFAULT_PASSWORD</span></p>
          </div>
        </div>
      </div>
    )
  }

  // ============================================================================
  // RENDER: MAIN APP
  // ============================================================================

  const canUploadDelete = user && (user.role === 'admin' || user.role === 'super_user')
  const isAdmin = user && user.role === 'admin'

  return (
    <div className="flex flex-col h-screen bg-gradient-to-br from-slate-900 to-slate-800">

      {/* ADMIN PANEL MODAL */}
      {showAdminPanel && isAdmin && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-slate-800 rounded-lg shadow-2xl border border-slate-700 w-full max-w-4xl max-h-[90vh] overflow-hidden flex flex-col">
            <div className="p-6 border-b border-slate-700 flex items-center justify-between">
              <div className="flex items-center gap-2">
                <button
                  onClick={() => { setAdminTab('users'); fetchAllUsers() }}
                  className={`px-4 py-2 rounded-lg font-semibold transition ${adminTab === 'users' ? 'bg-slate-600 text-white' : 'text-slate-400 hover:text-white hover:bg-slate-700'}`}
                >
                  Users
                </button>
                <button
                  onClick={() => { setAdminTab('backup'); fetchBackupList() }}
                  className={`px-4 py-2 rounded-lg font-semibold transition ${adminTab === 'backup' ? 'bg-slate-600 text-white' : 'text-slate-400 hover:text-white hover:bg-slate-700'}`}
                >
                  Backup
                </button>
              </div>
              <button onClick={toggleAdminPanel} className="text-slate-400 hover:text-white text-2xl">
                ✕
              </button>
            </div>

            <div className="flex-1 overflow-y-auto p-6 space-y-6">

              {/* USERS TAB */}
              {adminTab === 'users' && (
                <>
                  <div className="bg-slate-700 rounded-lg p-4">
                    <h3 className="text-lg font-semibold text-white mb-4">Create New User</h3>
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
                        <option value="user">User (read-only)</option>
                        <option value="super_user">Super User (upload/delete)</option>
                        <option value="admin">Admin (full access)</option>
                      </select>
                      <button
                        type="submit"
                        disabled={creatingUser}
                        className="col-span-2 py-2 bg-green-600 hover:bg-green-700 disabled:bg-slate-600 text-white font-semibold rounded transition"
                      >
                        {creatingUser ? 'Creating...' : 'Create User'}
                      </button>
                    </form>
                  </div>

                  <div>
                    <div className="flex items-center justify-between mb-4">
                      <h3 className="text-lg font-semibold text-white">Registered Users ({allUsers.length})</h3>
                      <button onClick={fetchAllUsers} className="text-sm text-blue-400 hover:text-blue-300">
                        Refresh
                      </button>
                    </div>

                    {loadingUsers ? (
                      <p className="text-center text-slate-400 py-8">Loading...</p>
                    ) : allUsers.length === 0 ? (
                      <p className="text-center text-slate-400 py-8">No users found</p>
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
                              {u.id !== user.id ? (
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
                                    Delete
                                  </button>
                                </>
                              ) : (
                                <span className="text-sm text-slate-400 italic">(You)</span>
                              )}
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                </>
              )}

              {/* BACKUP TAB */}
              {adminTab === 'backup' && (
                <div className="space-y-6">
                  <div className="bg-slate-700 rounded-lg p-4">
                    <div className="flex items-center justify-between mb-3">
                      <h3 className="text-lg font-semibold text-white">Create Backup</h3>
                      <button
                        onClick={handleRunBackup}
                        disabled={backupRunning}
                        className="px-4 py-2 bg-green-600 hover:bg-green-700 disabled:bg-slate-600 text-white rounded-lg transition font-semibold text-sm"
                      >
                        {backupRunning ? 'Running...' : 'Run Backup Now'}
                      </button>
                    </div>
                    <p className="text-sm text-slate-400">
                      Creates a tar.gz archive of the SQLite database and Qdrant snapshot.
                      Automatic backup runs daily at 02:00 UTC.
                    </p>
                  </div>

                  <div className="bg-slate-700 rounded-lg p-4">
                    <div className="flex items-center justify-between mb-3">
                      <h3 className="text-lg font-semibold text-white">Local Archives ({localBackups.length})</h3>
                      <button onClick={fetchBackupList} className="text-sm text-blue-400 hover:text-blue-300">
                        Refresh
                      </button>
                    </div>
                    {localBackups.length === 0 ? (
                      <p className="text-slate-400 text-sm">No backups yet</p>
                    ) : (
                      <div className="space-y-2 max-h-64 overflow-y-auto">
                        {localBackups.map((name, idx) => (
                          <div key={idx} className="bg-slate-600 rounded-lg p-3 flex items-center">
                            <span className="text-white text-sm font-mono truncate">{name}</span>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                </div>
              )}

            </div>
          </div>
        </div>
      )}

      {/* CHANGE PASSWORD MODAL */}
      {showChangePasswordModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-slate-800 rounded-lg shadow-2xl border border-slate-700 w-full max-w-md">
            <div className="p-6 border-b border-slate-700 flex items-center justify-between">
              <h2 className="text-2xl font-bold text-white">Change Password</h2>
              <button onClick={toggleChangePasswordModal} className="text-slate-400 hover:text-white text-2xl">✕</button>
            </div>

            <div className="p-6">
              <form onSubmit={handleChangePassword} className="space-y-4">
                <div>
                  <label className="block text-sm font-medium text-slate-300 mb-2">Current Password</label>
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
                  <label className="block text-sm font-medium text-slate-300 mb-2">New Password</label>
                  <input
                    type="password"
                    value={passwordForm.newPassword}
                    onChange={(e) => setPasswordForm({ ...passwordForm, newPassword: e.target.value })}
                    className="w-full px-3 py-2 bg-slate-700 text-white rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
                    placeholder="••••••••"
                    required
                    minLength={6}
                  />
                  <p className="text-xs text-slate-400 mt-1">Minimum 6 characters</p>
                </div>

                <div>
                  <label className="block text-sm font-medium text-slate-300 mb-2">Confirm New Password</label>
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
                    Cancel
                  </button>
                  <button
                    type="submit"
                    disabled={changingPassword}
                    className="flex-1 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-slate-600 text-white font-semibold rounded-lg transition"
                  >
                    {changingPassword ? 'Updating...' : 'Change Password'}
                  </button>
                </div>
              </form>
            </div>
          </div>
        </div>
      )}

      {/* HEADER */}
      <header className="bg-slate-800 border-b border-slate-700 px-6 py-4 flex items-center justify-between">
        <h1 className="text-2xl font-bold text-white">{BRANDING.clientName}</h1>

        <div className="flex items-center gap-6">
          <div className="flex items-center gap-2">
            <div className={`w-3 h-3 rounded-full ${
              status === 'ready' ? 'bg-green-500' :
              status === 'error' ? 'bg-red-500' :
              'bg-yellow-500'
            }`} />
            <span className="text-sm text-slate-300">
              {status === 'ready' ? 'Online' : status === 'error' ? 'Offline' : 'Checking...'}
            </span>
          </div>

          <div className="flex items-center gap-3 border-l border-slate-700 pl-6">
            <div className="text-right">
              <p className="text-sm font-semibold text-white">{user.username}</p>
              <p className={`text-xs ${
                user.role === 'admin' ? 'text-red-400' :
                user.role === 'super_user' ? 'text-purple-400' :
                'text-blue-400'
              }`}>
                {user.role === 'admin' ? 'Admin' : user.role === 'super_user' ? 'Super User' : 'User'}
              </p>
            </div>

            {isAdmin && (
              <button
                onClick={toggleAdminPanel}
                className="px-3 py-1 bg-slate-700 hover:bg-slate-600 text-white text-sm rounded transition"
                title="Admin panel"
              >
                Admin
              </button>
            )}

            <button
              onClick={toggleChangePasswordModal}
              className="px-3 py-1 bg-slate-700 hover:bg-slate-600 text-white text-sm rounded transition"
              title="Change password"
            >
              Password
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

        {/* SIDEBAR — CONVERSATIONS */}
        {showConversationsSidebar && (
          <aside className="w-64 bg-slate-800 border-r border-slate-700 flex flex-col">
            <div className="p-4 border-b border-slate-700">
              <button
                onClick={() => createNewConversation(undefined)}
                className="w-full py-2 px-4 bg-blue-600 hover:bg-blue-700 text-white rounded-lg font-semibold transition"
              >
                + New Chat
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
                  onClick={() => loadConversation(conv.id, undefined)}
                >
                  <span className="truncate flex-1 text-sm">{conv.title}</span>
                  {conversations.length > 1 && (
                    <button
                      onClick={(e) => { e.stopPropagation(); deleteConversation(conv.id) }}
                      className="opacity-0 group-hover:opacity-100 text-red-400 hover:text-red-300 text-xs ml-2"
                    >
                      ✕
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
                ◀ Close
              </button>
            </div>
          </aside>
        )}

        {/* CHAT AREA */}
        <main className="flex-1 flex flex-col min-w-0">
          {!showConversationsSidebar && (
            <button
              onClick={() => setShowConversationsSidebar(true)}
              className="absolute top-20 left-4 p-2 bg-slate-700 text-white rounded-lg shadow-lg hover:bg-slate-600 z-10"
            >
              ▶
            </button>
          )}

          <div className="flex-1 overflow-y-auto p-6 space-y-4">
            {messages.length === 0 ? (
              <div className="flex items-center justify-center h-full">
                <div className="text-center text-slate-400">
                  <h2 className="text-2xl font-bold mb-2">Hello, {user.username}!</h2>
                  <p>Start a conversation{canUploadDelete ? ' or upload documents to get started' : ''}.</p>
                </div>
              </div>
            ) : (
              messages.map((msg, idx) => (
                <div key={idx} className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}>
                  <div className={`max-w-3xl rounded-lg p-4 ${
                    msg.role === 'user'
                      ? 'bg-blue-600 text-white'
                      : msg.error
                      ? 'bg-red-900/30 border border-red-500 text-red-200'
                      : 'bg-slate-700 text-slate-100'
                  }`}>
                    <p className="whitespace-pre-wrap leading-relaxed">{msg.content}</p>

                    {msg.sources && msg.sources.length > 0 && (
                      <div className="mt-4 pt-4 border-t border-slate-600 space-y-2">
                        <p className="text-sm font-semibold text-slate-300">
                          Sources ({msg.sources.length}):
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
                              <span className="bg-green-600 text-white px-2 py-1 rounded text-xs font-bold flex-shrink-0">
                                {source.similarity != null ? (source.similarity * 100).toFixed(1) : 'N/A'}%
                              </span>
                            </div>
                          </div>
                        ))}
                      </div>
                    )}

                    <p className="text-xs text-slate-400 mt-2">
                      {new Date(msg.timestamp).toLocaleTimeString()}
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
                      <span>{isModelLoading ? 'LLM model is loading into memory...' : 'Searching...'}</span>
                    </div>
                    {isModelLoading && (
                      <p className="text-xs text-slate-400 ml-6">
                        May take 10–20 seconds on first start or after inactivity
                      </p>
                    )}
                  </div>
                </div>
              </div>
            )}

            <div ref={messagesEndRef} />
          </div>

          <div className="border-t border-slate-700 p-4 bg-slate-800">
            <form onSubmit={handleQuery} className="flex gap-2">
              <input
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Ask a question about your documents..."
                disabled={querying || status !== 'ready'}
                className="flex-1 bg-slate-700 text-white placeholder-slate-400 rounded-lg px-4 py-3 focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
              />
              <button
                type="submit"
                disabled={querying || !query.trim() || status !== 'ready'}
                className="px-6 py-3 bg-blue-600 hover:bg-blue-700 disabled:bg-slate-600 disabled:cursor-not-allowed text-white rounded-lg font-semibold transition"
              >
                {querying ? '...' : 'Send'}
              </button>
            </form>
          </div>
        </main>

        {/* SIDEBAR — DOCUMENTS */}
        {showDocumentsSidebar && (
          <aside className="w-80 bg-slate-800 border-l border-slate-700 flex flex-col">
            <div className="p-4 border-b border-slate-700">
              <h2 className="text-lg font-bold text-white mb-3">Documents</h2>

              {canUploadDelete && (
                <>
                  <input
                    ref={fileInputRef}
                    type="file"
                    onChange={handleFileUpload}
                    disabled={uploading}
                    className="hidden"
                    accept=".pdf,.docx,.txt,.doc,.pptx,.xlsx,.xls,.html,.htm"
                  />
                  <button
                    onClick={() => fileInputRef.current?.click()}
                    disabled={uploading}
                    className="w-full py-2 px-4 bg-green-600 hover:bg-green-700 disabled:bg-slate-600 text-white rounded-lg font-semibold transition"
                  >
                    {uploading ? `Uploading ${uploadProgress}%` : '+ Upload File'}
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
                        <p className="text-sm text-slate-300 text-center animate-pulse">{uploadPhase}</p>
                      )}
                    </div>
                  )}
                </>
              )}
            </div>

            <div className="flex-1 overflow-y-auto p-3">
              {loadingDocuments ? (
                <p className="text-center text-slate-400 py-4">Loading...</p>
              ) : documents.length === 0 ? (
                <p className="text-center text-slate-400 py-4 text-sm">No documents uploaded</p>
              ) : (
                <div className="space-y-2">
                  {documents.map((doc, idx) => (
                    <div key={idx} className="bg-slate-700 rounded-lg p-3 group hover:bg-slate-600 transition">
                      <div className="flex items-start justify-between gap-2">
                        <div className="flex-1 min-w-0">
                          <p className="text-white text-sm font-semibold truncate" title={doc.filename}>
                            {doc.filename}
                          </p>
                          <p className="text-slate-400 text-xs mt-1">
                            {doc.chunk_count || 0} chunks
                          </p>
                        </div>
                        {canUploadDelete && (
                          <button
                            onClick={() => handleDeleteDocument(doc.id)}
                            className="opacity-0 group-hover:opacity-100 text-red-400 hover:text-red-300 text-xs transition flex-shrink-0"
                            title="Delete document"
                          >
                            ✕
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
                Refresh
              </button>
              <button
                onClick={() => setShowDocumentsSidebar(false)}
                className="w-full py-1 text-xs text-slate-400 hover:text-white mt-1"
              >
                Close ▶
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
              {BRANDING.poweredBy} {BRANDING.poweredBySubtitle}
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
                alert('This system uses AI to analyze documents and provide answers. Results should be verified and do not replace professional advice.\n\n© I3K Technologies - All rights reserved.')
              }}
            >
              Disclaimer
            </a>
          </div>
        </div>
      </footer>
    </div>
  )
}

export default App
