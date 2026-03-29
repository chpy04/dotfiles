return {
	"neovim/nvim-lspconfig",
	version = "*",
	dependencies = {
		-- Mason
		{ "mason-org/mason.nvim", opts = {} },
		{
			"WhoIsSethDaniel/mason-tool-installer.nvim",
			opts = {
				ensure_installed = {
					-- LSP servers
					"lua-language-server",
					"pyright",
					"typescript-language-server",
					"zls",
					"ltex-ls",
					"marksman",
					"clangd",
					"rust-analyzer",
					"gopls",
					"bash-language-server",
					"jdtls",
					"prisma-language-server",
				-- Formatters
					"stylua",
					"prettier",
					"shfmt",
					"goimports",
					"gofumpt",
					"google-java-format",
					"latexindent",
                    -- Linters
                    "ruff",
					"eslint_d",
                    "shellcheck",
					-- DAP adapters
					"debugpy",
					"delve",
					"js-debug-adapter",
					"codelldb",
					"java-debug-adapter",
					"java-test",
				},
			},
		},
		-- Completion
		{ "hrsh7th/nvim-cmp", event = { "InsertEnter", "CmdlineEnter" } },
		"hrsh7th/cmp-nvim-lsp",
		"hrsh7th/cmp-buffer",
		"hrsh7th/cmp-path",
		"hrsh7th/cmp-cmdline",
		"saadparwaiz1/cmp_luasnip",
	},

	config = function()
		--------------------------------------------------------------------------
		-- CMP
		--------------------------------------------------------------------------
		local cmp = require("cmp")
		local cmp_lsp = require("cmp_nvim_lsp")

		cmp.setup({
			snippet = {
				expand = function(args)
					require("luasnip").lsp_expand(args.body)
				end,
			},
			mapping = cmp.mapping.preset.insert({
				["<C-p>"] = cmp.mapping.select_prev_item({ behavior = cmp.SelectBehavior.Select }),
				["<C-n>"] = cmp.mapping.select_next_item({ behavior = cmp.SelectBehavior.Select }),
				["<C-y>"] = cmp.mapping.confirm({ select = true }),
				["<C-Space>"] = cmp.mapping.complete(),
				["<Tab>"] = cmp.mapping(function(fallback)
					local copilot_ok, copilot = pcall(require, "copilot.suggestion")
					if copilot_ok and copilot.is_visible() then
						copilot.accept()
					elseif cmp.visible() then
						cmp.confirm({ select = true })
					elseif require("luasnip").expand_or_jumpable() then
						require("luasnip").expand_or_jump()
					else
						fallback()
					end
				end, { "i", "s" }),
				["<S-Tab>"] = cmp.mapping(function(fallback)
					if cmp.visible() then
						cmp.select_prev_item()
					elseif require("luasnip").jumpable(-1) then
						require("luasnip").jump(-1)
					else
						fallback()
					end
				end, { "i", "s" }),
			}),
			sources = cmp.config.sources({
				{ name = "nvim_lsp" },
				{ name = "luasnip" },
				{ name = "path" },
			}, { { name = "buffer" } }),
		})

		-- Cmdline completion for "/" and "?" (search)
		cmp.setup.cmdline({ "/", "?" }, {
			mapping = cmp.mapping.preset.cmdline(),
			sources = {
				{ name = "buffer" },
			},
		})

		-- Cmdline completion for ":" (command)
		cmp.setup.cmdline(":", {
			mapping = cmp.mapping.preset.cmdline(),
			sources = cmp.config.sources({
				{ name = "path" },
			}, {
				{ name = "cmdline" },
			}),
			matching = { disallow_symbol_nonprefix_matching = false },
		})

		local cmp_enabled = true
		vim.keymap.set("n", "<leader>ltc", function()
			cmp_enabled = not cmp_enabled
			cmp.setup({ enabled = cmp_enabled })
			vim.notify("Suggestions: " .. (cmp_enabled and "ENABLED" or "DISABLED"), vim.log.levels.INFO)
		end, { silent = true, desc = "Completion: toggle suggestions" })

		----------------------------------------------------------------------
		-- LSP core
		----------------------------------------------------------------------
		vim.lsp.config("*", {
			capabilities = cmp_lsp.default_capabilities(),
		})

		----------------------------------------------------------------------
		-- Server configs
		----------------------------------------------------------------------

		-- Lua
		vim.lsp.config("lua_ls", {
			filetypes = { "lua" },
			root_markers = { ".luarc.json", ".luarc.jsonc", ".luacheckrc", "stylua.toml", ".stylua.toml", ".git" },
			settings = {
				Lua = {
					runtime = { version = "LuaJIT" },
					diagnostics = { globals = { "bit", "vim", "it", "describe", "before_each", "after_each" } },
					workspace = { checkThirdParty = false },
					hint = { enable = true },
				},
			},
		})

		-- Python
		local function get_python_path(workspace)
			-- 1. Respect VIRTUAL_ENV if already activated
			local venv = vim.env.VIRTUAL_ENV
			if venv then
				return venv .. "/bin/python"
			end

			-- 2. Auto-detect .venv in project root
			local venv_path = workspace .. "/.venv/bin/python"
			if vim.fn.executable(venv_path) == 1 then
				return venv_path
			end

			-- 3. Fallback to system python
			return vim.fn.exepath("python3") or "python"
		end

		vim.lsp.config("pyright", {
			filetypes = { "python" },
			root_markers = {
				"pyrightconfig.json",
				"pyproject.toml",
				"setup.py",
				"setup.cfg",
				"requirements.txt",
				"Pipfile",
				".git",
			},
			on_init = function(client)
				local root = client.config.root_dir or vim.fn.getcwd()
				local python_path = get_python_path(root)
				client.config.settings.python.pythonPath = python_path
			end,
			settings = {
				python = {
					analysis = {
						typeCheckingMode = "standard",
						autoSearchPaths = true,
						useLibraryCodeForTypes = true,
					},
				},
			},
		})

		-- TypeScript/JavaScript
		vim.lsp.config("ts_ls", {
			filetypes = { "javascript", "javascriptreact", "typescript", "typescriptreact" },
			root_markers = { "package.json", "tsconfig.json", "jsconfig.json", ".git" },
			settings = {
				typescript = {
					inlayHints = {
						includeInlayParameterNameHints = "all",
						includeInlayParameterNameHintsWhenArgumentMatchesName = false,
						includeInlayFunctionParameterTypeHints = true,
						includeInlayVariableTypeHints = true,
						includeInlayVariableTypeHintsWhenTypeMatchesName = false,
						includeInlayPropertyDeclarationTypeHints = true,
						includeInlayFunctionLikeReturnTypeHints = true,
						includeInlayEnumMemberValueHints = true,
					},
				},
				javascript = {
					inlayHints = {
						includeInlayParameterNameHints = "all",
						includeInlayParameterNameHintsWhenArgumentMatchesName = false,
						includeInlayFunctionParameterTypeHints = true,
						includeInlayVariableTypeHints = true,
						includeInlayVariableTypeHintsWhenTypeMatchesName = false,
						includeInlayPropertyDeclarationTypeHints = true,
						includeInlayFunctionLikeReturnTypeHints = true,
						includeInlayEnumMemberValueHints = true,
					},
				},
			},
		})

		-- LTEX
		vim.lsp.config("ltex", {
			filetypes = { "tex", "latex", "bib" },
			root_markers = { ".git" },
			settings = {
				ltex = {
					checkFrequency = "save",
					language = "en-US",
					enabled = { "tex", "latex", "bib" },
					disabledRules = { ["en-US"] = { "WHITESPACE_RULE" } },
				},
			},
		})

		-- Zig
		vim.lsp.config("zls", {
			filetypes = { "zig", "zir" },
			root_markers = { "zls.json", "build.zig", ".git" },
			settings = {
				zls = {
					enable_inlay_hints = true,
					enable_snippets = true,
					warn_style = true,
				},
			},
		})

		-- Marksman
		vim.lsp.config("marksman", {
			filetypes = { "markdown" },
			root_markers = { ".git", ".marksman.toml", "index.md" },
		})

		-- C/C++
		vim.lsp.config("clangd", {
			filetypes = { "c", "cpp", "objc", "objcpp" },
			root_markers = { "compile_commands.json", "compile_flags.txt", ".clangd", ".git" },
		})

		-- Rust
		vim.lsp.config("rust_analyzer", {
			filetypes = { "rust" },
			root_markers = { "Cargo.toml", "rust-project.json", ".git" },
			settings = {
				["rust-analyzer"] = {
					checkOnSave = { command = "clippy" },
					cargo = { allFeatures = true },
				},
			},
		})

		-- Go
		vim.lsp.config("gopls", {
			filetypes = { "go", "gomod", "gowork", "gotmpl" },
			root_markers = { "go.mod", "go.work", ".git" },
			settings = {
				gopls = {
					analyses = {
						unusedparams = true,
						shadow = true,
					},
					staticcheck = true,
					gofumpt = true,
					hints = {
						assignVariableTypes = true,
						compositeLiteralFields = true,
						compositeLiteralTypes = true,
						constantValues = true,
						functionTypeParameters = true,
						parameterNames = true,
						rangeVariableTypes = true,
					},
				},
			},
		})

		-- Bash
		vim.lsp.config("bashls", {
			filetypes = { "sh", "bash" },
			root_markers = { ".git" },
		})

		-- Prisma
		vim.lsp.config("prismals", {
			filetypes = { "prisma" },
			root_markers = { ".git" },
		})

		-- Enable all Mason-managed servers
		vim.lsp.enable({
			"lua_ls",
			"pyright",
			"ts_ls",
			"zls",
			"ltex",
			"marksman",
			"clangd",
			"rust_analyzer",
			"gopls",
			"bashls",
			"prismals",
		})

		-- Racket (installed via raco, not Mason)
	local racket_lsp_group = vim.api.nvim_create_augroup("RacketLanguageServer", { clear = true })

	vim.api.nvim_create_autocmd("FileType", {
		group = racket_lsp_group,
		pattern = { "racket", "scheme" },
		callback = function()
			local root_dir = vim.fn.getcwd()
			local git = vim.fs.find(".git", { upward = true })[1]
			if git then
				root_dir = vim.fs.dirname(git)
			end

			vim.lsp.start({
				name = "racket_langserver",
				cmd = { "racket", "-l", "racket-langserver" },
				root_dir = root_dir,
			})
		end,
	})

	-- R (installed via R, not Mason)
		-- Use autocmd to explicitly start server since it's not managed by Mason
		local r_lsp_group = vim.api.nvim_create_augroup("RLanguageServer", { clear = true })

		vim.api.nvim_create_autocmd("FileType", {
			group = r_lsp_group,
			pattern = { "r" },
			callback = function()
				-- Find git root or use cwd
				local root_dir = vim.fn.getcwd()
				local git = vim.fs.find(".git", { upward = true })[1]
				if git then
					root_dir = vim.fs.dirname(git)
				end

				vim.lsp.start({
					name = "r_language_server",
					cmd = { "R", "--slave", "-e", "languageserver::run()" },
					root_dir = root_dir,
				})
			end,
		})
	end,
}
