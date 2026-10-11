- **The article lookup can be turned off.** The lookup of an article named
  by number — meant for collections of legislation whose files are named
  after the article they hold — now has a switch in the *Answers* tab of
  the admin panel (`GET`/`PUT /api/admin/settings/article-lookup`, admin
  only, setting `article_lookup_enabled`). On by default. A question that
  names no article skips it without reading the setting.
